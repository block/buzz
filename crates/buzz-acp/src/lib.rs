#![deny(unsafe_code)]

mod acp;
mod config;
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
use std::sync::Arc;
use std::time::Duration;

use acp::{AcpClient, EnvVar, McpServer};
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
use filter::SubscriptionRule;
use futures_util::FutureExt;
use nostr::{PublicKey, ToBech32};
use pool::{
    AcceptedDropControl, AgentPool, ControlSignal, IdleSwitchResult, ModelSwitchRequest,
    ModelSwitchRollback, OwnedAgent, PromptContext, PromptOutcome, PromptResult, PromptSource,
    SessionState, TimeoutKind,
};
use pool_lifecycle::PoolLifecycle;
use queue::{CancelReason, EnqueueOccurrenceId, EventQueue, FlushBatch, QueuedEvent, ThreadTags};
use relay::{HarnessRelay, RelayEventPublisher};
use tokio::sync::{mpsc, watch};
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
    match channel_info.resolve(channel_id).await {
        Some(info) => info.channel_type == "dm",
        None => {
            tracing::warn!(
                channel_id = %channel_id,
                "channel type unresolved — treating as DM for author gate (fail closed)"
            );
            true
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

    async fn wait_priority(&mut self) {
        let now = tokio::time::Instant::now();
        if self.next_publish > now {
            tokio::time::sleep_until(self.next_publish).await;
        }
        let published_at = tokio::time::Instant::now();
        self.published.push_back(published_at);
        self.next_publish = published_at + OBSERVER_PUBLISH_INTERVAL;
    }
}

fn spawn_relay_observer_publisher(
    observer: observer::ObserverHandle,
    publisher: RelayEventPublisher,
    keys: nostr::Keys,
    agent_pubkey_hex: String,
    owner_pubkey_hex: String,
    owner_pubkey: PublicKey,
) -> tokio::task::JoinHandle<bool> {
    tokio::spawn(async move {
        // Subscribe BEFORE snapshotting so an event emitted between the two
        // calls is never lost: it lands in the snapshot, the live receiver, or
        // both. The overlap is deduped in the run loop via exact snapshot
        // `seq` membership (monotonic, assigned at emit).
        let rx = observer.subscribe();
        let snapshot = observer.snapshot();
        // The publisher owns only the receiver. Retaining this sender across
        // the run-loop await would keep both broadcast lanes open forever,
        // making graceful shutdown time out even after the process-level
        // observer handle is dropped.
        drop(observer);
        run_relay_observer_publisher(
            snapshot,
            rx,
            publisher,
            keys,
            agent_pubkey_hex,
            owner_pubkey_hex,
            owner_pubkey,
        )
        .await
    })
}

async fn run_relay_observer_publisher(
    snapshot: Vec<observer::ObserverEvent>,
    mut rx: observer::ObserverReceiver,
    publisher: RelayEventPublisher,
    keys: nostr::Keys,
    agent_pubkey_hex: String,
    owner_pubkey_hex: String,
    owner_pubkey: PublicKey,
) -> bool {
    let mut terminal_delivery_ok = true;
    let mut coalescer = ObserverChunkCoalescer::default();
    let mut pacer = ObserverPublishPacer::new();
    let snapshot_seqs: HashSet<_> = snapshot.iter().map(|event| event.seq).collect();
    for event in snapshot {
        for event in coalescer.ingest(event) {
            terminal_delivery_ok &= publish_relay_observer_event_preemptible(
                &publisher,
                &keys,
                &agent_pubkey_hex,
                &owner_pubkey_hex,
                &owner_pubkey,
                &mut pacer,
                &mut rx,
                &snapshot_seqs,
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
                        if snapshot_seqs.contains(&event.seq) {
                            continue;
                        }
                        if event.kind == "control_result" {
                            // A terminal result received while chunks are
                            // coalescing must not be appended behind the
                            // coalescer's pending flush.
                            terminal_delivery_ok &= publish_relay_observer_event(
                                &publisher,
                                &keys,
                                &agent_pubkey_hex,
                                &owner_pubkey_hex,
                                &owner_pubkey,
                                &mut pacer,
                                event,
                            )
                            .await;
                            continue;
                        }
                        for event in coalescer.ingest(event) {
                            terminal_delivery_ok &= publish_relay_observer_event_preemptible(
                                &publisher, &keys, &agent_pubkey_hex,
                                &owner_pubkey_hex, &owner_pubkey, &mut pacer,
                                &mut rx, &snapshot_seqs, event,
                            ).await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        // This merged receiver can lag because either lane
                        // overflowed. Treat it as terminal-delivery uncertainty
                        // so shutdown cannot report verified proof delivery.
                        terminal_delivery_ok = false;
                        for event in coalescer.flush() {
                            terminal_delivery_ok &= publish_relay_observer_event_preemptible(
                                &publisher, &keys, &agent_pubkey_hex,
                                &owner_pubkey_hex, &owner_pubkey, &mut pacer,
                                &mut rx, &snapshot_seqs, event,
                            ).await;
                        }
                        tracing::warn!(dropped = count, "relay observer publisher lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        for event in coalescer.flush() {
                            terminal_delivery_ok &= publish_relay_observer_event_preemptible(
                                &publisher, &keys, &agent_pubkey_hex,
                                &owner_pubkey_hex, &owner_pubkey, &mut pacer,
                                &mut rx, &snapshot_seqs, event,
                            ).await;
                        }
                        break;
                    }
                }
            }
            _ = flush_interval.tick() => {
                // Periodic flush ensures live streaming even during continuous chunk delivery.
                for event in coalescer.flush() {
                    terminal_delivery_ok &= publish_relay_observer_event_preemptible(
                        &publisher, &keys, &agent_pubkey_hex,
                        &owner_pubkey_hex, &owner_pubkey, &mut pacer,
                        &mut rx, &snapshot_seqs, event,
                    ).await;
                }
            }
        }
    }
    terminal_delivery_ok
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
    event: observer::ObserverEvent,
) -> bool {
    let is_control_result = event.kind == "control_result";
    if is_control_result {
        pacer.wait_priority().await;
    } else {
        pacer.wait().await;
    }
    publish_relay_observer_event_now(
        publisher,
        keys,
        agent_pubkey_hex,
        owner_pubkey_hex,
        owner_pubkey,
        event,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn publish_relay_observer_event_preemptible(
    publisher: &RelayEventPublisher,
    keys: &nostr::Keys,
    agent_pubkey_hex: &str,
    owner_pubkey_hex: &str,
    owner_pubkey: &PublicKey,
    pacer: &mut ObserverPublishPacer,
    rx: &mut observer::ObserverReceiver,
    snapshot_seqs: &HashSet<u64>,
    event: observer::ObserverEvent,
) -> bool {
    if event.kind == "control_result" {
        return publish_relay_observer_event(
            publisher,
            keys,
            agent_pubkey_hex,
            owner_pubkey_hex,
            owner_pubkey,
            pacer,
            event,
        )
        .await;
    }

    let mut priority_lane_open = true;
    while priority_lane_open {
        tokio::select! {
            biased;
            result = rx.recv_control_result() => {
                match result {
                    Ok(priority) => {
                        // The subscribe-before-snapshot overlap can put a
                        // captured terminal result in both lanes.
                        if !snapshot_seqs.contains(&priority.seq)
                            && !publish_relay_observer_event(
                                publisher,
                                keys,
                                agent_pubkey_hex,
                                owner_pubkey_hex,
                                owner_pubkey,
                                pacer,
                                priority,
                            )
                            .await
                        {
                            return false;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        tracing::warn!(
                            dropped = count,
                            "relay observer priority publisher lagged"
                        );
                        return false;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        priority_lane_open = false;
                    }
                }
            }
            _ = pacer.wait() => {
                return publish_relay_observer_event_now(
                    publisher,
                    keys,
                    agent_pubkey_hex,
                    owner_pubkey_hex,
                    owner_pubkey,
                    event,
                )
                .await;
            }
        }
    }

    pacer.wait().await;
    publish_relay_observer_event_now(
        publisher,
        keys,
        agent_pubkey_hex,
        owner_pubkey_hex,
        owner_pubkey,
        event,
    )
    .await
}

async fn publish_relay_observer_event_now(
    publisher: &RelayEventPublisher,
    keys: &nostr::Keys,
    agent_pubkey_hex: &str,
    owner_pubkey_hex: &str,
    owner_pubkey: &PublicKey,
    mut event: observer::ObserverEvent,
) -> bool {
    let is_control_result = event.kind == "control_result";
    // Trim oversized frames to fit the plaintext cap rather than letting
    // encrypt_observer_payload reject and drop them whole (silent telemetry loss).
    fit_observer_event_to_budget(&mut event);
    let encrypted = match encrypt_observer_payload(keys, owner_pubkey, &event) {
        Ok(encrypted) => encrypted,
        Err(error) => {
            tracing::warn!("failed to encrypt relay observer event: {error}");
            return !is_control_result;
        }
    };
    let mut builder = match buzz_sdk::build_agent_observer_frame(
        owner_pubkey_hex,
        agent_pubkey_hex,
        OBSERVER_FRAME_TELEMETRY,
        &encrypted,
    ) {
        Ok(builder) => builder,
        Err(error) => {
            tracing::warn!("failed to build relay observer event: {error}");
            return !is_control_result;
        }
    };
    if is_control_result {
        let priority_tag = match nostr::Tag::parse(["priority", "control-result"]) {
            Ok(tag) => tag,
            Err(error) => {
                tracing::warn!("failed to build observer control-result priority tag: {error}");
                return false;
            }
        };
        builder = builder.tag(priority_tag);
    }
    let signed = match builder.sign_with_keys(keys) {
        Ok(event) => event,
        Err(error) => {
            tracing::warn!("failed to sign relay observer event: {error}");
            return !is_control_result;
        }
    };
    let publish_result = if is_control_result {
        publisher.publish_terminal_event(signed).await
    } else {
        publisher.publish_event(signed).await
    };
    if let Err(error) = publish_result {
        tracing::warn!("relay observer event dropped: {error}");
        return !is_control_result;
    }
    true
}

/// Maximum age (seconds) for an observer control frame to be considered fresh.
const OBSERVER_CONTROL_FRESHNESS_SECS: i64 = 300;
const MODEL_SWITCH_REQUEST_ID_LEN: usize = 32;
/// Bound replay memory while covering substantially more than the maximum
/// number of legitimate controls expected inside the freshness window.
const OBSERVER_CONTROL_DEDUP_CAPACITY: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObserverControlAdmission {
    Admitted,
    Replay,
    CapacityExceeded,
}

struct ObserverControlDedup {
    capacity: usize,
    order: VecDeque<(String, i64)>,
    seen: HashSet<String>,
}

impl ObserverControlDedup {
    fn new(capacity: usize) -> Self {
        assert!(
            capacity > 0,
            "observer control dedup capacity must be positive"
        );
        Self {
            capacity,
            order: VecDeque::with_capacity(capacity),
            seen: HashSet::with_capacity(capacity),
        }
    }

    fn admit(
        &mut self,
        event_id: String,
        event_timestamp: i64,
        now: i64,
    ) -> ObserverControlAdmission {
        let seen = &mut self.seen;
        self.order.retain(|(retained_id, retained_timestamp)| {
            let fresh = retained_timestamp.abs_diff(now) <= OBSERVER_CONTROL_FRESHNESS_SECS as u64;
            if !fresh {
                seen.remove(retained_id);
            }
            fresh
        });

        if self.seen.contains(&event_id) {
            return ObserverControlAdmission::Replay;
        }
        if self.order.len() >= self.capacity {
            return ObserverControlAdmission::CapacityExceeded;
        }
        self.seen.insert(event_id.clone());
        self.order.push_back((event_id, event_timestamp));
        ObserverControlAdmission::Admitted
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_relay_observer_control_event(
    keys: &nostr::Keys,
    event: nostr::Event,
    dedup: &mut ObserverControlDedup,
    pool: &mut AgentPool,
    queue: &mut EventQueue,
    config: &Config,
    crash_history: &mut [SlotCircuit],
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<&observer::ObserverHandle>,
    owner_pubkey_hex: &str,
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
    if event_ts.abs_diff(now) > OBSERVER_CONTROL_FRESHNESS_SECS as u64 {
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
    match dedup.admit(event.id.to_hex(), event_ts, now) {
        ObserverControlAdmission::Admitted => {}
        ObserverControlAdmission::Replay => {
            tracing::warn!(event_id = %event.id, "replayed observer control frame — dropping");
            return;
        }
        ObserverControlAdmission::CapacityExceeded => {
            tracing::error!(
                event_id = %event.id,
                capacity = dedup.capacity,
                "observer control replay window is full — failing closed"
            );
            return;
        }
    }

    let command_type = payload.get("type").and_then(|value| value.as_str());
    match command_type {
        Some("cancel_turn") => {
            handle_cancel_turn_control(&payload, pool, observer);
        }
        Some("switch_model") => {
            handle_switch_model_control(
                &payload,
                pool,
                queue,
                config,
                crash_history,
                respawn_tx,
                respawn_tasks,
                observer,
            );
        }
        _ => {
            tracing::debug!(payload = %payload, "ignoring unknown observer control frame");
        }
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

    let status = match signal_in_flight_task(pool, channel_id, ControlSignal::Cancel) {
        ControlSignalResult::Delivered => "sent",
        ControlSignalResult::DropRecorded => "drop_recorded",
        ControlSignalResult::NotAccepted => "no_active_turn",
    };
    if let Some(observer) = observer {
        observer.emit(
            "control_result",
            None,
            &observer::ObserverContext {
                channel_id: Some(channel_id.to_string()),
                session_id: None,
                turn_id: None,
                started_at: None,
            },
            serde_json::json!({
                "type": "cancel_turn",
                "status": status,
            }),
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
/// Idle path: validate against the cached catalog, claim the owning adapter,
/// and recycle its process. The override takes effect on the fresh process's
/// next session without locally abandoning the prior remote session.
#[allow(clippy::too_many_arguments)]
fn handle_switch_model_control(
    payload: &serde_json::Value,
    pool: &mut AgentPool,
    queue: &mut EventQueue,
    config: &Config,
    crash_history: &mut [SlotCircuit],
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
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
    let Some(request_id) = parse_model_switch_request_id(payload) else {
        tracing::warn!("observer switch_model control frame missing or invalid requestId");
        return;
    };
    let request = ModelSwitchRequest::new(model_id, request_id);

    // A turn is in flight for this channel iff a task_map entry exists. The
    // agent is moved out of the pool during a turn, so the control oneshot is
    // the only reachable lever; an idle channel has no such entry.
    let in_flight_agent = pool
        .task_map()
        .values()
        .find(|m| m.channel_id == Some(channel_id))
        .map(|m| m.agent_index);

    let status = if let Some(agent_index) = in_flight_agent {
        // Busy path: deliver over the oneshot. `false` means the oneshot was
        // already consumed this turn (a prior cancel/interrupt) — the turn is
        // already ending, so the switch cannot land on it.
        if signal_in_flight_task(pool, channel_id, ControlSignal::SwitchModel(request))
            == ControlSignalResult::Delivered
        {
            queue.require_agent(channel_id, agent_index);
            "sent"
        } else {
            "turn_ending"
        }
    } else {
        // Idle path: validate before taking the adapter out of service.
        match pool.switch_idle_agent_model(channel_id, model_id, request_id) {
            IdleSwitchResult::Recycle(agent) => {
                let agent = *agent;
                let agent_index = agent.index;
                queue.require_agent(channel_id, agent_index);
                let scheduled = spawn_model_switch_recycle_task(
                    agent,
                    config,
                    &mut crash_history[agent_index],
                    respawn_tx,
                    respawn_tasks,
                    observer.cloned(),
                    PendingSwitchControl {
                        channel_id,
                        model_id: model_id.to_string(),
                        request_id: request_id.to_string(),
                    },
                );
                if scheduled {
                    // Scheduling a recycle is acceptance, not proof that the
                    // replacement initialized or applied this model.
                    "recycling"
                } else {
                    queue.clear_required_agent(channel_id);
                    "switch_failed"
                }
            }
            IdleSwitchResult::UnsupportedModel => "unsupported_model",
            IdleSwitchResult::NoIdleAgent => "no_active_turn",
        }
    };

    if let Some(observer) = observer {
        observer.emit(
            "control_result",
            None,
            &observer::ObserverContext {
                channel_id: Some(channel_id.to_string()),
                session_id: None,
                turn_id: None,
                started_at: None,
            },
            switch_model_control_result_payload(status, model_id, request_id),
        );
    }
}

fn switch_model_control_result_payload(
    status: &str,
    model_id: &str,
    request_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "type": "switch_model",
        "status": status,
        "modelId": model_id,
        "requestId": request_id,
    })
}

fn parse_model_switch_request_id(payload: &serde_json::Value) -> Option<&str> {
    let request_id = payload.get("requestId")?.as_str()?;
    (request_id.len() == MODEL_SWITCH_REQUEST_ID_LEN
        && request_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()))
    .then_some(request_id)
}

#[allow(clippy::too_many_arguments)]
fn recycle_idle_channel_sessions(
    pool: &mut AgentPool,
    channel_id: Uuid,
    config: &Config,
    crash_history: &mut [SlotCircuit],
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<observer::ObserverHandle>,
) -> usize {
    let agents = pool.take_idle_agents_with_session(channel_id);
    let count = agents.len();
    for agent in agents {
        let index = agent.index;
        let _ = spawn_recycle_task(
            agent,
            config,
            &mut crash_history[index],
            respawn_tx,
            respawn_tasks,
            observer.clone(),
        );
    }
    count
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
/// Minimum spacing between negotiated compatibility recycles for one slot.
///
/// Optional-close adapters require process replacement to retire sessions, but
/// normal cancellation or max-token responses must not become an unbounded
/// kill/spawn loop.
const RECYCLE_MIN_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
struct ReplacementModelIntent {
    desired_model: Option<String>,
    model_overridden: bool,
    model_switch_request_id: Option<String>,
    model_switch_rollback: Option<Box<ModelSwitchRollback>>,
}

#[derive(Clone, Debug)]
struct PendingSwitchControl {
    channel_id: Uuid,
    model_id: String,
    request_id: String,
}

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
    /// Permanent, process-lifetime quarantine after an adapter's termination
    /// could not be proved. A timed crash cooldown must never authorize a new
    /// owner while the prior process group may still exist.
    cleanup_unverified: bool,
    /// True while a background respawn/refill task is in flight for this slot.
    /// Prevents duplicate spawns from maintenance ticks that fire before the
    /// previous spawn_and_init completes.
    respawn_in_flight: bool,
    /// Earliest time another negotiated compatibility recycle may begin.
    next_recycle_not_before: Option<std::time::Instant>,
    /// Live model intent that must survive an intentional or cleanup-triggered
    /// replacement until a replacement agent initializes successfully.
    pending_model_intent: Option<ReplacementModelIntent>,
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
    fn new() -> Self {
        Self {
            crash_times: Vec::new(),
            open_until: None,
            cleanup_unverified: false,
            respawn_in_flight: false,
            next_recycle_not_before: None,
            pending_model_intent: None,
        }
    }

    /// Reserve the next compatibility-recycle slot and return its delay.
    ///
    /// This budget is separate from crash accounting: negotiated recycling is
    /// healthy maintenance, but it still needs a hard rate bound.
    fn schedule_recycle(&mut self) -> Duration {
        let now = std::time::Instant::now();
        let scheduled = self.next_recycle_not_before.unwrap_or(now).max(now);
        self.next_recycle_not_before = Some(scheduled + RECYCLE_MIN_INTERVAL);
        scheduled.saturating_duration_since(now)
    }

    /// Record a crash and decide whether to respawn.
    ///
    /// This is the **single canonical path** for all crash → respawn decisions.
    /// Called by `respawn_agent_into`, `recover_panicked_agent`, and slot refill.
    fn record_crash(&mut self) -> CrashVerdict {
        if self.cleanup_unverified {
            return CrashVerdict::CircuitOpen;
        }
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

    /// Quarantine this slot for the remainder of the supervisor process.
    ///
    /// Recovery requires an operator-visible process restart after external
    /// process/cgroup verification; an ordinary cooldown is not cleanup proof.
    fn mark_cleanup_unverified(&mut self) {
        self.cleanup_unverified = true;
        self.open_until = None;
    }

    /// Restore the exact intent that preceded a terminally rejected switch.
    ///
    /// The observer result makes the current requested model final from the
    /// caller's perspective, so a later cooldown refill must never silently
    /// retry it. Nested rollback history represents an older pending switch
    /// and remains eligible to resume.
    fn rollback_terminal_switch_failure(&mut self) {
        let rollback = self
            .pending_model_intent
            .take()
            .and_then(|intent| intent.model_switch_rollback);
        self.pending_model_intent = rollback.map(|rollback| ReplacementModelIntent {
            desired_model: rollback.desired_model,
            model_overridden: rollback.model_overridden,
            model_switch_request_id: rollback.request_id,
            model_switch_rollback: rollback.previous,
        });
    }

    fn blocks_supervisor_exit(&self) -> bool {
        self.cleanup_unverified
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
        if self.cleanup_unverified {
            return false;
        }
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

/// True if any slot has recovery work in flight or is deliberately quarantined.
/// Used to prevent premature "all agents dead" exits: respawns may succeed in
/// seconds, while quarantined slots must keep the current process alive and
/// degraded so a service manager cannot restart into possible process overlap.
fn any_respawn_in_flight(crash_history: &[SlotCircuit]) -> bool {
    crash_history
        .iter()
        .any(|s| s.respawn_in_flight || s.blocks_supervisor_exit())
}

/// Result of a background respawn task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RespawnFailureClass {
    /// The prior owner was retired, but replacement spawn/init failed.
    Spawn,
    /// The prior owner's direct child/process group could not be proved absent.
    CleanupUnverified,
}

/// Typed failure from spawning and initializing a replacement adapter.
///
/// Cleanup proof is part of the outcome: a failed handshake is retryable only
/// when the newly spawned client was also proved shut down.
#[derive(Debug)]
enum SpawnInitFailure {
    Spawn(anyhow::Error),
    CleanupUnverified(anyhow::Error),
}

impl SpawnInitFailure {
    fn into_respawn_failure(self) -> (anyhow::Error, RespawnFailureClass) {
        match self {
            Self::Spawn(error) => (error, RespawnFailureClass::Spawn),
            Self::CleanupUnverified(error) => (error, RespawnFailureClass::CleanupUnverified),
        }
    }
}

type SpawnInitResult = std::result::Result<(AcpClient, u32, String), SpawnInitFailure>;

struct RespawnResult {
    index: usize,
    /// `Some`: initialized replacement; `None`: old adapter was fully shut
    /// down while an open circuit intentionally suppressed replacement.
    result: Result<Option<(AcpClient, u32, String)>>,
    failure_class: Option<RespawnFailureClass>,
    /// Model intent to apply to the replacement agent.
    ///
    /// Crash recovery supplies configured defaults. Intentional compatibility
    /// recycling preserves a live `SwitchModel` override.
    desired_model: Option<String>,
    model_overridden: bool,
    /// Preserve the slot's pending model intent if replacement did not
    /// initialize. Cleanup-triggered replacement and negotiated recycling set
    /// this; ordinary crash recovery intentionally resets to config defaults.
    retain_model_intent: bool,
    /// True only when this result already emitted a terminal `switch_failed`
    /// control result. The supervisor must atomically restore the preceding
    /// intent before it permits any cooldown refill.
    terminal_switch_failure: bool,
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
    occurrence_id: EnqueueOccurrenceId,
    event_id: String,
    /// `Ok` if the read loop sent any of the locked `SteerAck` variants.
    /// `Err` if the oneshot was dropped without a send — should not happen
    /// under the current read-loop drains, but if it ever does the main
    /// loop treats it as `PromptCompletedNeutral` (release withheld, no
    /// fallback signal) to avoid leaking the withheld event.
    ack: std::result::Result<pool::SteerAck, tokio::sync::oneshot::error::RecvError>,
}

type InitializingReplacement = Arc<tokio::sync::Mutex<Option<AcpClient>>>;

#[derive(Clone)]
enum RespawnPhase {
    /// The task may still own the previous adapter process.
    OldOwnerUnverified,
    /// Prior-owner cleanup is verified and no replacement process exists.
    CleanNoOwner,
    /// A replacement process exists and must be explicitly shut down if the
    /// surrounding respawn task is cancelled.
    Initializing(InitializingReplacement),
}

/// RAII guard that ensures a `RespawnResult` is sent even if the task panics.
/// Without this, a panicked respawn task would leave `respawn_in_flight = true`
/// permanently, silently losing the slot forever.
struct RespawnGuard {
    index: usize,
    tx: mpsc::Sender<RespawnResult>,
    desired_model: Option<String>,
    model_overridden: bool,
    retain_model_intent: bool,
    switch_failure: Option<(observer::ObserverHandle, PendingSwitchControl)>,
    terminal_switch_failure_emitted: bool,
    phase: RespawnPhase,
    sent: bool,
}

impl RespawnGuard {
    fn new(
        index: usize,
        tx: mpsc::Sender<RespawnResult>,
        desired_model: Option<String>,
        model_overridden: bool,
        retain_model_intent: bool,
    ) -> Self {
        Self {
            index,
            tx,
            desired_model,
            model_overridden,
            retain_model_intent,
            switch_failure: None,
            terminal_switch_failure_emitted: false,
            phase: RespawnPhase::OldOwnerUnverified,
            sent: false,
        }
    }

    fn mark_owner_cleanup_verified(&mut self) {
        self.phase = RespawnPhase::CleanNoOwner;
    }

    fn mark_replacement_initializing(&mut self, client: InitializingReplacement) {
        self.phase = RespawnPhase::Initializing(client);
    }

    fn with_switch_failure(
        mut self,
        observer: Option<observer::ObserverHandle>,
        control: Option<PendingSwitchControl>,
    ) -> Self {
        self.switch_failure = observer.zip(control);
        self
    }

    fn emit_switch_failure(&mut self) -> bool {
        if self.terminal_switch_failure_emitted {
            return true;
        }
        let Some((observer, control)) = self.switch_failure.take() else {
            return false;
        };
        observer.emit(
            "control_result",
            Some(self.index),
            &observer::ObserverContext {
                channel_id: Some(control.channel_id.to_string()),
                session_id: None,
                turn_id: None,
                started_at: None,
            },
            serde_json::json!({
                "type": "switch_model",
                "status": "switch_failed",
                "modelId": control.model_id,
                "requestId": control.request_id,
            }),
        );
        self.terminal_switch_failure_emitted = true;
        true
    }

    /// Send the result and disarm the guard. Uses `try_send` (sync) so there
    /// is no await boundary between marking `sent` and actually enqueueing —
    /// cancellation cannot slip between the two.
    fn send(mut self, result: SpawnInitResult) {
        match result {
            Ok(initialized) => self.send_inner(Ok(Some(initialized)), None),
            Err(failure) => {
                let (error, failure_class) = failure.into_respawn_failure();
                self.send_inner(Err(error), Some(failure_class));
            }
        }
    }

    /// Report that bounded shutdown completed and an open circuit deliberately
    /// suppressed replacement.
    fn send_cleanup_complete(mut self) {
        self.send_inner(Ok(None), None);
    }

    /// Report that replacement is forbidden because prior-owner termination
    /// could not be proved.
    fn send_cleanup_unverified(mut self, error: anyhow::Error) {
        self.send_inner(Err(error), Some(RespawnFailureClass::CleanupUnverified));
    }

    fn send_inner(
        &mut self,
        result: Result<Option<(AcpClient, u32, String)>>,
        failure_class: Option<RespawnFailureClass>,
    ) {
        let terminal_switch_failure = result.is_err() && self.emit_switch_failure();
        // Invariant: try_send succeeds because the channel capacity equals the
        // slot count, and respawn_in_flight guarantees at most one outstanding
        // result per slot. If this ever fails, the channel sizing or the
        // respawn_in_flight guard has drifted — that's a bug, not a transient.
        match self.tx.try_send(RespawnResult {
            index: self.index,
            result,
            failure_class,
            desired_model: self.desired_model.clone(),
            model_overridden: self.model_overridden,
            retain_model_intent: self.retain_model_intent,
            terminal_switch_failure,
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
        if self.sent {
            return;
        }

        match self.phase.clone() {
            RespawnPhase::OldOwnerUnverified => {
                tracing::error!(
                    agent = self.index,
                    "respawn task exited while process ownership was unverified"
                );
                self.send_inner(
                    Err(anyhow::anyhow!("respawn task panicked or was cancelled")),
                    Some(RespawnFailureClass::CleanupUnverified),
                );
            }
            RespawnPhase::CleanNoOwner => {
                tracing::debug!(
                    agent = self.index,
                    "respawn task cancelled after verified cleanup and before replacement spawn"
                );
                self.send_inner(Ok(None), None);
            }
            RespawnPhase::Initializing(client) => {
                let Some(runtime) = tokio::runtime::Handle::try_current().ok() else {
                    self.send_inner(
                        Err(anyhow::anyhow!(
                            "replacement initialization cancelled without an async cleanup runtime"
                        )),
                        Some(RespawnFailureClass::CleanupUnverified),
                    );
                    return;
                };

                // Transfer result reporting and any terminal switch context to
                // a detached bounded-cleanup task. The cancelled initializer
                // releases the mutex guard while unwinding; this task then
                // takes the replacement client and proves its process group is
                // absent before classifying cancellation as retryable.
                let mut cleanup_guard = RespawnGuard::new(
                    self.index,
                    self.tx.clone(),
                    self.desired_model.clone(),
                    self.model_overridden,
                    self.retain_model_intent,
                )
                .with_switch_failure(None, None);
                cleanup_guard.switch_failure = self.switch_failure.take();
                cleanup_guard.terminal_switch_failure_emitted =
                    self.terminal_switch_failure_emitted;
                self.sent = true;
                drop(runtime.spawn(async move {
                    let mut slot = client.lock().await;
                    let replacement = slot.take();
                    drop(slot);
                    match replacement {
                        Some(mut acp) => match acp.shutdown().await {
                            Ok(()) => {
                                cleanup_guard.send(Err(SpawnInitFailure::Spawn(anyhow::anyhow!(
                                    "replacement initialization cancelled after verified cleanup"
                                ))))
                            }
                            Err(error) => cleanup_guard.send_cleanup_unverified(anyhow::anyhow!(
                                "replacement initialization cancelled; cleanup unverified: {error}"
                            )),
                        },
                        None => cleanup_guard.send_cleanup_complete(),
                    }
                }));
            }
        }
    }
}

#[cfg(test)]
mod respawn_guard_tests {
    use super::*;

    #[cfg(unix)]
    struct ProcessGroupCleanup(Option<nix::unistd::Pid>);

    #[cfg(unix)]
    impl Drop for ProcessGroupCleanup {
        fn drop(&mut self) {
            if let Some(process_group) = self.0 {
                let _ = nix::sys::signal::killpg(process_group, nix::sys::signal::Signal::SIGKILL);
            }
        }
    }

    #[tokio::test]
    async fn cancelled_respawn_task_quarantines_slot_when_cleanup_is_unverified() {
        let (tx, mut rx) = mpsc::channel(1);
        let (armed_tx, armed_rx) = tokio::sync::oneshot::channel();

        let task = tokio::spawn(async move {
            let _guard = RespawnGuard::new(1, tx, None, false, false);
            armed_tx.send(()).expect("test task should arm guard");
            std::future::pending::<()>().await;
        });

        armed_rx.await.expect("test task should start");
        task.abort();
        assert!(task
            .await
            .expect_err("task should be cancelled")
            .is_cancelled());

        let result = rx
            .recv()
            .await
            .expect("guard drop must report the cancelled respawn");
        assert!(result.result.is_err());
        assert_eq!(
            result.failure_class,
            Some(RespawnFailureClass::CleanupUnverified)
        );
    }

    #[tokio::test]
    async fn cancellation_after_verified_old_owner_cleanup_is_clean() {
        let (tx, mut rx) = mpsc::channel(1);
        let (armed_tx, armed_rx) = tokio::sync::oneshot::channel();

        let task = tokio::spawn(async move {
            let mut guard = RespawnGuard::new(1, tx, None, false, false);
            guard.mark_owner_cleanup_verified();
            armed_tx.send(()).expect("test task should arm guard");
            std::future::pending::<()>().await;
        });

        armed_rx.await.expect("test task should start");
        task.abort();
        assert!(task
            .await
            .expect_err("task should be cancelled")
            .is_cancelled());

        let result = rx
            .recv()
            .await
            .expect("guard drop must report clean cancellation");
        assert!(matches!(result.result, Ok(None)));
        assert_eq!(result.failure_class, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_during_replacement_initialize_verifies_process_group_cleanup() {
        use nix::errno::Errno;
        use nix::sys::signal::{killpg, Signal};
        use nix::unistd::Pid;

        let pid_file = std::env::temp_dir().join(format!(
            "buzz-acp-respawn-init-{}-{}.pid",
            std::process::id(),
            Uuid::new_v4()
        ));
        let script = r#"printf '%s\n' "$$" > "$1"; exec /bin/sleep 300"#;
        let args = vec![
            "-c".to_string(),
            script.to_string(),
            "buzz-acp-respawn-init-test".to_string(),
            pid_file.to_string_lossy().into_owned(),
        ];
        let (tx, mut rx) = mpsc::channel(1);
        let task = tokio::spawn(async move {
            let mut guard = RespawnGuard::new(1, tx, None, false, false);
            guard.mark_owner_cleanup_verified();
            let result = spawn_and_init("bash", &args, &[], false, 1, None, &mut guard).await;
            guard.send(result);
        });

        tokio::time::timeout(Duration::from_secs(10), async {
            while !pid_file.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("replacement adapter must publish its PID");
        let process_group = Pid::from_raw(
            std::fs::read_to_string(&pid_file)
                .expect("replacement PID file")
                .trim()
                .parse()
                .expect("replacement PID"),
        );
        let mut cleanup = ProcessGroupCleanup(Some(process_group));
        assert_eq!(killpg(process_group, None::<Signal>), Ok(()));

        task.abort();
        assert!(task
            .await
            .expect_err("initializer task should be cancelled")
            .is_cancelled());
        let result = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("bounded cleanup must report")
            .expect("cleanup result channel");
        assert!(result.result.is_err());
        assert_eq!(
            result.failure_class,
            Some(RespawnFailureClass::Spawn),
            "verified replacement cleanup remains retryable"
        );
        assert_eq!(
            killpg(process_group, None::<Signal>),
            Err(Errno::ESRCH),
            "cancelled replacement process group must be absent before reporting"
        );
        cleanup.0 = None;
        let _ = std::fs::remove_file(pid_file);
    }

    #[test]
    fn partial_init_cleanup_failure_is_not_downgraded_to_spawn_failure() {
        let (tx, mut rx) = mpsc::channel(1);
        let guard = RespawnGuard::new(1, tx, None, false, false);

        guard.send(Err(SpawnInitFailure::CleanupUnverified(anyhow::anyhow!(
            "spawned adapter cleanup unverified"
        ))));

        let result = rx
            .try_recv()
            .expect("typed spawn/init failure must reach the supervisor");
        assert!(result.result.is_err());
        assert_eq!(
            result.failure_class,
            Some(RespawnFailureClass::CleanupUnverified)
        );
    }

    #[test]
    fn failed_idle_switch_recycle_emits_channel_scoped_terminal_failure() {
        let (tx, mut rx) = mpsc::channel(1);
        let observer = observer::ObserverHandle::in_process();
        let channel_id = Uuid::new_v4();
        let guard = RespawnGuard::new(2, tx, Some("model-b".into()), true, true)
            .with_switch_failure(
                Some(observer.clone()),
                Some(PendingSwitchControl {
                    channel_id,
                    model_id: "model-b".into(),
                    request_id: "0123456789abcdef0123456789abcdef".into(),
                }),
            );

        guard.send(Err(SpawnInitFailure::Spawn(anyhow::anyhow!(
            "replacement failed"
        ))));

        let result = rx.try_recv().expect("failure must reach the supervisor");
        assert!(result.result.is_err());
        assert_eq!(
            result.failure_class,
            Some(RespawnFailureClass::Spawn),
            "a normally returned spawn/init failure remains eligible for cooldown retry"
        );
        assert!(
            result.terminal_switch_failure,
            "the supervisor must know that the request received a terminal result"
        );
        let failures: Vec<_> = observer
            .snapshot()
            .into_iter()
            .filter(|event| event.kind == "control_result")
            .collect();
        let channel_id_string = channel_id.to_string();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].agent_index, Some(2));
        assert_eq!(
            failures[0].channel_id.as_deref(),
            Some(channel_id_string.as_str())
        );
        assert_eq!(failures[0].payload["status"], "switch_failed");
        assert_eq!(failures[0].payload["modelId"], "model-b");
        assert_eq!(
            failures[0].payload["requestId"],
            "0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn terminal_switch_failure_restores_exact_prior_intent_for_retry() {
        let mut slot = SlotCircuit::new();
        slot.pending_model_intent = Some(ReplacementModelIntent {
            desired_model: Some("model-b".into()),
            model_overridden: true,
            model_switch_request_id: Some("request-b".into()),
            model_switch_rollback: Some(Box::new(ModelSwitchRollback {
                desired_model: Some("model-a".into()),
                model_overridden: true,
                request_id: Some("request-a".into()),
                previous: Some(Box::new(ModelSwitchRollback {
                    desired_model: Some("configured-default".into()),
                    model_overridden: false,
                    request_id: None,
                    previous: None,
                })),
            })),
        });

        slot.rollback_terminal_switch_failure();

        let restored = slot
            .pending_model_intent
            .as_ref()
            .expect("the exact prior pending intent must resume");
        assert_eq!(restored.desired_model.as_deref(), Some("model-a"));
        assert!(restored.model_overridden);
        assert_eq!(
            restored.model_switch_request_id.as_deref(),
            Some("request-a")
        );
        let prior = restored
            .model_switch_rollback
            .as_ref()
            .expect("nested rollback history must remain intact");
        assert_eq!(prior.desired_model.as_deref(), Some("configured-default"));
        assert!(!prior.model_overridden);
        assert_eq!(prior.request_id, None);
        assert!(prior.previous.is_none());
    }
}

#[cfg(test)]
mod pool_startup_tests {
    use super::*;

    #[test]
    fn cleanup_unverified_lazy_startup_is_not_retried() {
        assert!(
            PoolStartupFailure::CleanupUnverified("cleanup unverified".into()).cleanup_unverified()
        );
        assert!(!PoolStartupFailure::Retryable("provider unavailable".into()).cleanup_unverified());
        assert!(lazy_pool_can_wake(true, false, false));
        assert!(!lazy_pool_can_wake(true, false, true));
        assert!(!lazy_pool_can_wake(false, false, false));
        assert!(!lazy_pool_can_wake(true, true, false));
    }

    #[test]
    fn cleanup_unverified_eager_startup_enters_non_spawning_quarantine() {
        let startup = resolve_initial_pool_startup(
            false,
            Some(Err(PoolStartupFailure::CleanupUnverified(
                "adapter ownership is unverified".into(),
            ))),
        )
        .expect("cleanup uncertainty must not terminate the supervisor");

        assert!(
            matches!(startup, InitialPoolStartup::CleanupQuarantine(_)),
            "eager cleanup uncertainty must keep the harness alive without a ready pool"
        );
    }
}

#[cfg(test)]
mod shutdown_verification_tests {
    use super::*;

    #[test]
    fn checked_out_respawn_and_idle_uncertainty_each_prevent_clean_shutdown() {
        for owner in [
            ShutdownOwner::CheckedOut,
            ShutdownOwner::Respawn,
            ShutdownOwner::Idle,
        ] {
            let mut verification = ShutdownVerification::default();
            verification.record(owner, false);
            assert!(
                verification.into_result().is_err(),
                "{owner:?} ownership uncertainty must make shutdown fail"
            );
        }

        let mut verification = ShutdownVerification::default();
        verification.record(ShutdownOwner::CheckedOut, true);
        verification.record(ShutdownOwner::Respawn, true);
        verification.record(ShutdownOwner::Idle, true);
        assert!(
            verification.into_result().is_ok(),
            "fully verified ownership may return clean"
        );
    }

    #[tokio::test]
    async fn pre_loop_failure_cannot_escape_existing_cleanup_quarantine() {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        let task = tokio::spawn(async move {
            let mut pool = AgentPool::from_slots(Vec::new());
            await_pre_loop_operation(
                async { Err::<(), _>("relay unavailable") },
                &mut shutdown_rx,
                &mut pool,
                "relay connect",
                true,
            )
            .await
        });

        tokio::task::yield_now().await;
        assert!(
            !task.is_finished(),
            "an automatic pre-loop error must remain inside cleanup quarantine"
        );

        shutdown_tx
            .send(())
            .expect("explicit shutdown must release quarantine");
        let error = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("quarantined failure must respond to explicit shutdown")
            .expect("test task")
            .expect_err("pre-loop operation must still fail");
        assert!(
            error.to_string().contains("cleanup remains unverified"),
            "the returned error must preserve cleanup uncertainty: {error}"
        );
    }

    #[test]
    fn cli_failure_preserves_cleanup_uncertainty_as_primary_error() {
        let result: Result<()> = cli_failure_after_cleanup(
            anyhow::anyhow!("adapter protocol failed"),
            Err(anyhow::anyhow!("adapter cleanup unverified")),
        );

        let error = result.expect_err("cleanup uncertainty must fail the command");
        assert_eq!(error.to_string(), "adapter cleanup unverified");
    }
}

//
// Sync env-var propagation must run before the tokio runtime starts so that
// any child processes inherit the correct environment. This must happen in the
// sync entry point — `std::env::set_var` is only safe before tokio spawns
// worker threads (Rust 2024 edition safety requirement).

pub fn run() -> Result<()> {
    config::propagate_legacy_env_vars();
    tokio_main()
}

async fn install_shutdown_channel() -> Result<(watch::Sender<()>, watch::Receiver<()>)> {
    let (shutdown_tx, shutdown_rx) = watch::channel(());

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        // Constructing the streams installs Tokio's process-level handlers
        // synchronously. Do this before any adapter spawn so SIGINT/SIGTERM can
        // never take the default exit path while a detached adapter PGID lives.
        let mut sigint = signal(SignalKind::interrupt()).map_err(|error| anyhow::anyhow!(error))?;
        let mut sigterm =
            signal(SignalKind::terminate()).map_err(|error| anyhow::anyhow!(error))?;
        let signal_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            loop {
                let received = tokio::select! {
                    signal = sigint.recv() => signal,
                    signal = sigterm.recv() => signal,
                };
                if received.is_none() {
                    break;
                }
                let _ = signal_tx.send(());
            }
        });
    }

    #[cfg(not(unix))]
    {
        let signal_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            loop {
                if tokio::signal::ctrl_c().await.is_err() {
                    break;
                }
                let _ = signal_tx.send(());
            }
        });
        // Give the spawned ctrl_c future one poll so its platform handler is
        // registered before the caller is allowed to spawn an adapter.
        tokio::task::yield_now().await;
    }

    Ok((shutdown_tx, shutdown_rx))
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
        let (_shutdown_tx, shutdown_rx) = install_shutdown_channel().await?;
        return run_models(args, shutdown_rx).await;
    }

    if is_subcommand("auth-methods") {
        let filtered: Vec<String> = std::env::args()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, a)| a)
            .collect();
        let args = AuthMethodsArgs::parse_from(&filtered);
        let (_shutdown_tx, shutdown_rx) = install_shutdown_channel().await?;
        return run_auth_methods(args, shutdown_rx).await;
    }

    if is_subcommand("authenticate") {
        let filtered: Vec<String> = std::env::args()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, a)| a)
            .collect();
        let args = AuthenticateArgs::parse_from(&filtered);
        let (_shutdown_tx, shutdown_rx) = install_shutdown_channel().await?;
        return run_authenticate(args, shutdown_rx).await;
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

    let (shutdown_tx, mut shutdown_rx) = install_shutdown_channel().await?;

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

    let eager_pool_result = if config.lazy_pool {
        None
    } else {
        Some(
            initialize_agent_pool(
                &PoolStartup::from_config(&config, observer.clone()),
                Some(shutdown_rx.clone()),
            )
            .await,
        )
    };
    let (mut pool, mut pool_ready, mut pool_cleanup_unverified, initial_cleanup_error) =
        match resolve_initial_pool_startup(config.lazy_pool, eager_pool_result)? {
            InitialPoolStartup::Ready(pool) => (pool, true, false, None),
            InitialPoolStartup::Dormant => (
                AgentPool::from_slots((0..config.agents).map(|_| None).collect()),
                false,
                false,
                None,
            ),
            InitialPoolStartup::CleanupQuarantine(error) => {
                tracing::error!(
                    error,
                    "eager pool cleanup could not be verified — entering non-spawning quarantine"
                );
                (
                    AgentPool::from_slots((0..config.agents).map(|_| None).collect()),
                    false,
                    true,
                    Some(error),
                )
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

    // Resolve one canonical privileged owner before the relay background task
    // starts. This closes the handshake-buffer window where a configured
    // fallback owner could otherwise bypass relay freshness/replay admission.
    // Priority: BUZZ_AUTH_TAG (NIP-OA attestation) → --agent-owner flag.
    let startup_owner: Option<String> = resolve_agent_owner(&config);
    if let Some(ref owner) = startup_owner {
        tracing::info!("agent owner: {owner}");
    } else {
        tracing::info!("no agent owner configured");
    }

    // Parse BUZZ_AUTH_TAG into a nostr::Tag for NIP-OA relay membership delegation.
    let relay_auth_tag: Option<nostr::Tag> = std::env::var("BUZZ_AUTH_TAG")
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| buzz_sdk::nip_oa::parse_auth_tag(&s).ok());

    let mut relay = await_pre_loop_operation(
        HarnessRelay::connect_with_owner(
            &config.relay_url,
            &config.keys,
            &pubkey_hex,
            relay_auth_tag,
            startup_owner.clone(),
        ),
        &mut shutdown_rx,
        &mut pool,
        "relay connect",
        pool_cleanup_unverified,
    )
    .await?;

    // Tell the relay background task the watermark so it can use
    // `since = watermark - 5s` on the first REQ instead of `since=now`.
    // Best-effort: a failure here is non-fatal (we just lose the startup window
    // protection, which is the same as the pre-fix behaviour).
    if let Err(e) = relay.set_startup_watermark(startup_watermark).await {
        tracing::warn!("failed to set startup watermark: {e}");
    }

    tracing::info!("connected to relay at {}", config.relay_url);

    await_pre_loop_operation(
        relay.subscribe_membership_notifications(),
        &mut shutdown_rx,
        &mut pool,
        "membership notification subscribe",
        pool_cleanup_unverified,
    )
    .await?;
    tracing::info!("subscribed to membership notifications");

    let presence_publisher = relay.event_publisher();
    let presence_keys = config.keys.clone();

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
    let mut observer_control_dedup = ObserverControlDedup::new(OBSERVER_CONTROL_DEDUP_CAPACITY);
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
                    await_pre_loop_operation(
                        relay.subscribe_observer_controls(),
                        &mut shutdown_rx,
                        &mut pool,
                        "observer control subscribe",
                        pool_cleanup_unverified,
                    )
                    .await?;
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

    let channel_info_map = await_pre_loop_operation(
        relay.discover_channels(),
        &mut shutdown_rx,
        &mut pool,
        "channel discovery",
        pool_cleanup_unverified,
    )
    .await?;

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
            match config::load_rules(&config.config_path) {
                Ok(rules) => rules,
                Err(error) => {
                    return fail_pre_loop_with_pool_cleanup(
                        &mut pool,
                        &mut shutdown_rx,
                        "subscription rule loading",
                        anyhow::anyhow!(error),
                        pool_cleanup_unverified,
                        false,
                    )
                    .await;
                }
            }
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
    let mut queue =
        EventQueue::new(dedup_mode).with_in_flight_deadline(config.max_turn_duration_secs);

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
    } else if let Some(error) = initial_cleanup_error.as_deref() {
        emit_runtime_lifecycle(
            observer.as_ref(),
            &runtime_start_nonce,
            &pubkey_hex,
            &config.relay_url,
            "failed",
            Some(error),
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
        permission_mode: config.permission_mode,
        agent_keys: config.keys.clone(),
        agent_owner_pubkey: startup_owner
            .as_deref()
            .and_then(|hex| nostr::PublicKey::from_hex(hex).ok()),
        memory_enabled: config.memory_enabled,
        harness_name: crate::config::normalize_agent_command_identity(&config.agent_command),
        relay_url: config.relay_url.clone(),
    });

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
    let mut typing_channels: HashMap<Uuid, ThreadTags> = HashMap::new();
    let mut presence_task: Option<tokio::task::JoinHandle<()>> = None;

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
    let (wake_tx, mut wake_rx) =
        mpsc::channel::<(u32, std::result::Result<AgentPool, PoolStartupFailure>)>(1);
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
        .map(|_| SlotCircuit::new())
        .collect();

    //
    // Branches 1 & 2 both need to borrow `pool`, but they access different
    // fields (result_rx vs join_set). We use `rx_and_join_set()` to split the
    // borrow, yielding a typed enum so the outer code can dispatch cleanly.
    enum PoolEvent {
        Result(Box<PromptResult>),
        Panic(tokio::task::JoinError),
        SteerAck(SteerAckEvent),
        Wake(u32, std::result::Result<AgentPool, PoolStartupFailure>),
    }

    loop {
        // Whether buffered work is waiting on a lazy pool. Also gates the
        // retry-deadline sleep arm below: a `Failed` lifecycle keeps its
        // (possibly past) `retry_at` until the next wake, so sleeping on it
        // unconditionally would complete instantly on every iteration — a
        // busy spin — whenever the queued work drained after a failed wake.
        let mut lazy_wake_work_pending = false;
        if lazy_pool_can_wake(config.lazy_pool, pool_ready, pool_cleanup_unverified) {
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
                    let result = initialize_agent_pool(&startup, Some(wake_shutdown)).await;
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
                let pending_intent = slot.pending_model_intent.clone();
                let (desired_model, model_overridden, retain_model_intent) = match pending_intent {
                    Some(intent) => (intent.desired_model, intent.model_overridden, true),
                    None => (config.model.clone(), false, false),
                };
                let mut guard = RespawnGuard::new(
                    idx,
                    respawn_tx.clone(),
                    desired_model,
                    model_overridden,
                    retain_model_intent,
                );
                respawn_tasks.spawn(async move {
                    // This refill starts from an already-empty slot.
                    guard.mark_owner_cleanup_verified();
                    let result =
                        spawn_and_init(&cmd, &args, &env, has_codex, idx, observer, &mut guard)
                            .await;
                    guard.send(result);
                });
            }

            // Flush requeued batches whose retry_after has expired. Without
            // this, a batch requeued during crash recovery can sit idle
            // indefinitely on quiet channels — dispatch_pending is only
            // called on relay events or pool results, neither of which
            // arrive when the channel is silent.
            if queue.has_flushable_work() {
                for (channel_id, thread_tags) in dispatch_pending(&mut pool, &mut queue, &ctx) {
                    typing_channels.insert(channel_id, thread_tags);
                }
            }
        }

        // Reap every completed replacement task before entering the biased
        // select. The select arm below is the quiet-loop wake path; this eager
        // drain prevents sustained ready relay/result traffic from starving
        // that lower-priority arm and retaining completed JoinSet entries.
        while let Some(completed) = respawn_tasks.try_join_next() {
            if let Err(error) = completed {
                // RespawnGuard::Drop already enqueues the matching failure.
                tracing::error!("respawn task failed: {error}");
            }
        }

        let mut respawn_collected = false;
        while let Ok(rr) = respawn_rx.try_recv() {
            let slot = &mut crash_history[rr.index];
            slot.respawn_in_flight = false;
            match rr.result {
                Ok(Some((acp, protocol_version, agent_name))) => {
                    let pending_model_intent = slot.pending_model_intent.take();
                    let model_switch_request_id = pending_model_intent
                        .as_ref()
                        .and_then(|intent| intent.model_switch_request_id.clone());
                    let model_switch_rollback =
                        pending_model_intent.and_then(|intent| intent.model_switch_rollback);
                    let agent = OwnedAgent {
                        index: rr.index,
                        acp,
                        state: SessionState::default(),
                        model_capabilities: None,
                        desired_model: rr.desired_model,
                        model_overridden: rr.model_overridden,
                        model_switch_request_id,
                        model_switch_rollback,
                        agent_name,
                        goose_system_prompt_supported: None,
                        protocol_version,
                    };
                    pool.return_agent(agent);
                    queue.release_required_agent(rr.index);
                    tracing::info!(agent = rr.index, "respawn complete");
                    respawn_collected = true;
                }
                Ok(None) => {
                    if !rr.retain_model_intent {
                        slot.pending_model_intent = None;
                    }
                    tracing::warn!(
                        agent = rr.index,
                        "old adapter shutdown complete; circuit remains open"
                    );
                }
                Err(e) => {
                    if rr.terminal_switch_failure {
                        slot.rollback_terminal_switch_failure();
                    } else if !rr.retain_model_intent {
                        slot.pending_model_intent = None;
                    }
                    if rr.failure_class == Some(RespawnFailureClass::Spawn) {
                        slot.mark_spawn_failed();
                        tracing::warn!(agent = rr.index, "respawn failed: {e} — circuit re-opened");
                    } else {
                        // Missing classification also fails closed. Internal
                        // bookkeeping ambiguity is not cleanup proof.
                        slot.mark_cleanup_unverified();
                        tracing::error!(
                            agent = rr.index,
                            "adapter cleanup unverified: {e} — slot quarantined until process restart"
                        );
                    }
                }
            }
        }
        // Flush requeued events that were waiting for a live agent. Without
        // this, batches requeued during crash recovery sit idle until the
        // next relay event arrives — which can be minutes on quiet channels.
        if respawn_collected {
            for (channel_id, thread_tags) in dispatch_pending(&mut pool, &mut queue, &ctx) {
                typing_channels.insert(channel_id, thread_tags);
            }
        }

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
                        await_automatic_exit_permission(
                            &mut shutdown_rx,
                            cleanup_ownership_unverified(
                                pool_cleanup_unverified,
                                &crash_history,
                            ),
                            "result channel closed",
                        )
                        .await;
                        break;
                    }
                },
                // Guard: join_next() returns None immediately when JoinSet is
                // empty, which would cause a tight spin. Only poll when there
                // are in-flight tasks.
                Some(Err(e)) = join_set.join_next(), if !join_set.is_empty() => {
                    Some(PoolEvent::Panic(e))
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
                Some(Err(error)) = wake_tasks.join_next(), if !wake_tasks.is_empty() => {
                    // A panicked/cancelled startup task may have dropped a
                    // partially spawned client without verified shutdown.
                    // Keep this supervisor process from attempting another
                    // lazy wake.
                    pool_cleanup_unverified = true;
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
                completed = respawn_tasks.join_next(), if !respawn_tasks.is_empty() => {
                    if let Some(Err(error)) = completed {
                        // RespawnGuard::Drop already sends the matching failure
                        // result, so this arm only reports and reaps the task.
                        tracing::error!("respawn task failed: {error}");
                    }
                    None
                }
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(
                    last_maintenance + maintenance_interval
                )), if pool_ready => {
                    // Wake the loop so cooldown probes, retry throttles, and
                    // empty-slot refill progress even when every optional
                    // heartbeat and the relay are quiet.
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
                                    &mut observer_control_dedup,
                                    &mut pool,
                                    &mut queue,
                                    &config,
                                    &mut crash_history,
                                    &respawn_tx,
                                    &mut respawn_tasks,
                                    observer.as_ref(),
                                    owner_hex,
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
                                let ts = buzz_event.event.created_at.as_secs();
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
                                    // Drain queued events and recycle idle adapters that
                                    // still own the removed channel. Checked-out adapters
                                    // are recycled when they return.
                                    let drained_ids = queue.drain_channel(ch);
                                    let recycled = if pool_ready {
                                        recycle_idle_channel_sessions(
                                            &mut pool,
                                            ch,
                                            &config,
                                            &mut crash_history,
                                            &respawn_tx,
                                            &mut respawn_tasks,
                                            observer.clone(),
                                        )
                                    } else {
                                        0
                                    };
                                    // Track removed channels so checked-out adapters are
                                    // recycled instead of locally forgetting ownership.
                                    removed_channels.insert(ch);
                                    typing_channels.remove(&ch);
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
                                    if !drained_ids.is_empty() || recycled > 0 {
                                        tracing::info!(
                                            channel_id = %ch,
                                            drained = drained_ids.len(),
                                            recycled,
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

                            // Check: kind:9, content "!shutdown", from owner, mentions THIS agent.
                            let is_shutdown = is_admitted_owner_control_command(
                                &buzz_event.event,
                                kind_u32,
                                "!shutdown",
                                &pubkey_hex,
                                buzz_event.privileged_control_admitted,
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
                            let is_cancel = is_admitted_owner_control_command(
                                &buzz_event.event,
                                kind_u32,
                                "!cancel",
                                &pubkey_hex,
                                buzz_event.privileged_control_admitted,
                            );
                            if is_cancel {
                                if let Some(owner) = owner_cache.get() {
                                    if buzz_event.event.pubkey.to_hex() == *owner {
                                        let control_result = signal_in_flight_task(
                                            &mut pool,
                                            buzz_event.channel_id,
                                            ControlSignal::Cancel,
                                        );
                                        match control_result {
                                            ControlSignalResult::Delivered => {}
                                            ControlSignalResult::DropRecorded => tracing::info!(
                                                channel_id = %buzz_event.channel_id,
                                                "!cancel received after a prior control — batch drop recorded"
                                            ),
                                            ControlSignalResult::NotAccepted => tracing::warn!(
                                                channel_id = %buzz_event.channel_id,
                                                "!cancel received but no in-flight task — no-op"
                                            ),
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
                            let is_rotate = is_admitted_owner_control_command(
                                &buzz_event.event,
                                kind_u32,
                                "!rotate",
                                &pubkey_hex,
                                buzz_event.privileged_control_admitted,
                            );
                            if is_rotate {
                                if let Some(owner) = owner_cache.get() {
                                    if buzz_event.event.pubkey.to_hex() == *owner {
                                        let control_result = signal_in_flight_task(
                                            &mut pool,
                                            buzz_event.channel_id,
                                            ControlSignal::Rotate,
                                        );
                                        match control_result {
                                            ControlSignalResult::Delivered => tracing::info!(
                                                channel_id = %buzz_event.channel_id,
                                                "!rotate received — cancelling in-flight turn and rotating session"
                                            ),
                                            ControlSignalResult::DropRecorded => tracing::info!(
                                                channel_id = %buzz_event.channel_id,
                                                "!rotate received after a prior control — batch drop and rotation recorded"
                                            ),
                                            ControlSignalResult::NotAccepted => {
                                                let recycled = recycle_idle_channel_sessions(
                                                    &mut pool,
                                                    buzz_event.channel_id,
                                                    &config,
                                                    &mut crash_history,
                                                    &respawn_tx,
                                                    &mut respawn_tasks,
                                                    observer.clone(),
                                                );
                                                tracing::info!(
                                                    channel_id = %buzz_event.channel_id,
                                                    recycled,
                                                    "!rotate received — recycling idle channel session owner(s)"
                                                );
                                            }
                                        }
                                        continue; // consume event — do NOT push to queue
                                    }
                                }
                                // Not from owner — fall through to normal prompt handling.
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
                                let is_dm =
                                    is_dm_channel(buzz_event.channel_id, &ctx.channel_info).await;
                                let allowed = author_allowed(
                                    &config.respond_to,
                                    &config.respond_to_allowlist,
                                    &author,
                                    is_dm,
                                    &owner_cache,
                                    &ctx.rest_client,
                                )
                                .await;
                                if !allowed {
                                    tracing::debug!(
                                        channel_id = %buzz_event.channel_id,
                                        author = %buzz_event.event.pubkey.to_hex(),
                                        mode = %config.respond_to,
                                        is_dm,
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
                            let occurrence_id = queue.push(QueuedEvent {
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
                            if occurrence_id.is_some() {
                                let rc = ctx.rest_client.clone();
                                let eid = event_id_hex.clone();
                                tokio::spawn(async move {
                                    pool::reaction_add(&rc, &eid, "👀").await;
                                });
                            }
                            // Event is already queued. If mode requires it AND
                            // the channel has an in-flight task, fire cancel —
                            // OR take the non-cancelling (ACP steer) fork for Steer signals.
                            if let Some(occurrence_id) = occurrence_id {
                                if queue.is_channel_in_flight(buzz_event.channel_id) {
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
                                            buzz_event.channel_id,
                                            occurrence_id,
                                            event_for_steer,
                                            prompt_tag_for_steer,
                                            &steer_ack_tx,
                                        );
                                    if !native_attempted {
                                        signal_in_flight_task(
                                            &mut pool,
                                            buzz_event.channel_id,
                                            signal,
                                        );
                                    }
                                }
                                }
                            }
                            if pool_ready {
                                for (channel_id, thread_tags) in
                                    dispatch_pending(&mut pool, &mut queue, &ctx)
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
                                await_automatic_exit_permission(
                                    &mut shutdown_rx,
                                    cleanup_ownership_unverified(
                                        pool_cleanup_unverified,
                                        &crash_history,
                                    ),
                                    "relay reconnect failed",
                                )
                                .await;
                                break;
                            }
                        }
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
                            dispatch_pending(&mut pool, &mut queue, &ctx)
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
                    for (&ch, thread_tags) in &typing_channels {
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
                // Stop typing indicator for the completed channel.
                if let PromptSource::Channel(ch) = &result.source {
                    typing_channels.remove(ch);
                }
                if handle_prompt_result(
                    &mut pool,
                    &mut queue,
                    &config,
                    *result,
                    &mut heartbeat_in_flight,
                    &removed_channels,
                    &mut crash_history,
                    &respawn_tx,
                    &mut respawn_tasks,
                    observer.clone(),
                    Some(&ctx.rest_client),
                ) == LoopAction::Exit
                {
                    await_automatic_exit_permission(
                        &mut shutdown_rx,
                        cleanup_ownership_unverified(pool_cleanup_unverified, &crash_history),
                        "prompt result requested exit",
                    )
                    .await;
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
                    Some(&ctx.rest_client),
                ) == LoopAction::Exit
                {
                    await_automatic_exit_permission(
                        &mut shutdown_rx,
                        cleanup_ownership_unverified(pool_cleanup_unverified, &crash_history),
                        "drained prompt result requested exit",
                    )
                    .await;
                    break;
                }
                for (channel_id, thread_tags) in dispatch_pending(&mut pool, &mut queue, &ctx) {
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
                    Some(&ctx.rest_client),
                );
                if pool.live_count() == 0 && !any_respawn_in_flight(&crash_history) {
                    tracing::error!("all agents dead — exiting");
                    await_automatic_exit_permission(
                        &mut shutdown_rx,
                        cleanup_ownership_unverified(pool_cleanup_unverified, &crash_history),
                        "all agents dead",
                    )
                    .await;
                    break;
                }
                for (channel_id, thread_tags) in dispatch_pending(&mut pool, &mut queue, &ctx) {
                    typing_channels.insert(channel_id, thread_tags);
                }
            }
            Some(PoolEvent::SteerAck(SteerAckEvent {
                channel_id,
                occurrence_id,
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
                    queue.extend_in_flight_deadline(channel_id, config.max_turn_duration_secs);
                }
                if drop_withheld {
                    queue.remove_native_steer(channel_id, occurrence_id);
                }
                if release_withheld {
                    queue.release_native_steer(channel_id, occurrence_id);
                }
                if signal_fallback {
                    // Universal cancel+merge fallback. Note: the
                    // queued event has already been released to the
                    // front of `queues[channel_id]`, so the cancel
                    // will pick it up as part of the merged batch and
                    // re-prompt the agent.
                    signal_in_flight_task(&mut pool, channel_id, ControlSignal::Steer);
                }
                // After releasing a withheld event, give dispatch a chance
                // to re-flush. If the prompt is still in flight, the
                // channel stays `in_flight_channels` and `flush_next`
                // skips it — but a Steer fallback signal sent above will
                // tear down the in-flight task; on its completion the
                // queue drains. We still try here in case the in-flight
                // task has already returned.
                for (channel_id, thread_tags) in dispatch_pending(&mut pool, &mut queue, &ctx) {
                    typing_channels.insert(channel_id, thread_tags);
                }
            }
            Some(PoolEvent::Wake(attempt, result)) => {
                let cleanup_unverified = result
                    .as_ref()
                    .err()
                    .is_some_and(PoolStartupFailure::cleanup_unverified);
                if cleanup_unverified {
                    pool_cleanup_unverified = true;
                }
                let result = result.map_err(|error| error.to_string());
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
                            dispatch_pending(&mut pool, &mut queue, &ctx)
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

    // Drain wake tasks gracefully rather than aborting: an in-flight
    // initialize_agent_pool observes the shutdown watch at its biased per-slot
    // select and reaps its partially-spawned agents itself. `shutdown()` here
    // would abort the task mid-init and drop those AcpClients via best-effort
    // Drop — the exact zombie class the eager path's spawn-outside-the-timeout
    // comment exists to prevent. Fire the watch first so exits that bypass the
    // signal handlers (result channel closed, LoopAction::Exit) cancel the wake
    // just as promptly. Timeout is a backstop for a slot stuck outside the
    // select (e.g. in spawn); only then do we fall back to aborting.
    let mut shutdown_verification = ShutdownVerification::default();
    shutdown_verification.record(ShutdownOwner::Wake, !pool_cleanup_unverified);
    shutdown_verification.record(
        ShutdownOwner::Respawn,
        !crash_history.iter().any(|slot| slot.cleanup_unverified),
    );
    let _ = shutdown_tx.send(());
    let wake_drain = tokio::time::timeout(Duration::from_secs(30), async {
        while wake_tasks.join_next().await.is_some() {}
    })
    .await;
    if wake_drain.is_err() {
        tracing::warn!("wake task did not drain within grace period — aborting");
        shutdown_verification.record(ShutdownOwner::Wake, false);
        wake_tasks.shutdown().await;
    }
    while let Ok((_attempt, result)) = wake_rx.try_recv() {
        match result {
            Ok(mut awakened_pool) => {
                let verified = shutdown_agent_pool(&mut awakened_pool).await;
                shutdown_verification.record(ShutdownOwner::Wake, verified);
            }
            Err(PoolStartupFailure::CleanupUnverified(_)) => {
                shutdown_verification.record(ShutdownOwner::Wake, false);
            }
            Err(PoolStartupFailure::Retryable(_)) => {}
        }
    }

    let signalled = pool.cancel_checked_out_for_shutdown();
    tracing::info!(signalled, "shutdown: cancelling in-flight prompts");
    // Preserve the typed process owner until every task returns it. Aborting a
    // task drops the only AcpClient handle and makes reaping/process-group
    // absence impossible to prove.
    let (rx_ref, js_ref) = pool.rx_and_join_set();
    let mut checked_out_cleanup_verified = true;
    loop {
        tokio::select! {
            result = js_ref.join_next() => {
                match result {
                    Some(Err(e)) => {
                        tracing::warn!("task error during shutdown: {e}");
                        checked_out_cleanup_verified = false;
                    }
                    Some(Ok(())) => {}
                    None => break,
                }
            }
            maybe_result = rx_ref.recv() => {
                if let Some(mut pr) = maybe_result {
                    let idx = pr.agent.index;
                    if shutdown_acp_with_log(
                        &mut pr.agent.acp,
                        Some(idx),
                        "checked-out agent shutdown",
                    )
                    .await
                    {
                        tracing::debug!(agent = idx, "reaped checked-out agent on shutdown");
                    } else {
                        checked_out_cleanup_verified = false;
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                // A repeated SIGINT/SIGTERM does not abort the task and lose
                // its sole typed process owner. Grace has already been skipped:
                // every returned owner is immediately hard-shut down, reaped,
                // and process-group-probed in the result arm above.
                tracing::warn!(
                    "repeated shutdown signal received — hard cleanup active; \
                     preserving typed ownership until verification completes"
                );
            }
        }
    }
    // Drain results that raced with the final join completion.
    while let Ok(mut pr) = pool.result_rx_try_recv() {
        let idx = pr.agent.index;
        if shutdown_acp_with_log(
            &mut pr.agent.acp,
            Some(idx),
            "late checked-out agent shutdown",
        )
        .await
        {
            tracing::debug!(agent = idx, "reaped late-arriving agent on shutdown");
        } else {
            checked_out_cleanup_verified = false;
        }
    }
    shutdown_verification.record(ShutdownOwner::CheckedOut, checked_out_cleanup_verified);
    // Explicitly shut down idle agents still sitting in their slots.
    let mut idle_cleanup_verified = true;
    for slot in pool.agents_mut().iter_mut() {
        if let Some(agent) = slot.take() {
            let idx = agent.index;
            let mut acp = agent.acp;
            if shutdown_acp_with_log(&mut acp, Some(idx), "idle agent shutdown").await {
                tracing::debug!(agent = idx, "reaped idle agent on shutdown");
            } else {
                idle_cleanup_verified = false;
            }
        }
    }
    shutdown_verification.record(ShutdownOwner::Idle, idle_cleanup_verified);
    drop(pool);

    // Track the exact slots whose ownership result has not reached the
    // supervisor. JoinSet length is not a safe proxy: a task can enqueue its
    // result and remain briefly joinable, which would make shutdown wait for a
    // second result that will never exist.
    let mut pending_respawn_results: HashSet<usize> = crash_history
        .iter()
        .enumerate()
        .filter_map(|(index, slot)| slot.respawn_in_flight.then_some(index))
        .collect();
    let mut shutdown_respawn_results = Vec::new();
    while let Ok(result) = respawn_rx.try_recv() {
        pending_respawn_results.remove(&result.index);
        shutdown_respawn_results.push(result);
    }

    // Abort any in-flight respawn tasks. Backoff-phase guards report a clean
    // no-owner cancellation synchronously. Initializing guards transfer their
    // AcpClient to a bounded cleanup task and report only after shutdown proof.
    respawn_tasks.shutdown().await;

    // AcpClient::shutdown may use two five-second child waits plus one
    // five-second process-group absence probe. All replacement cleanups run
    // concurrently, so one shared 16-second deadline covers their bounded
    // verification path.
    let cleanup_deadline = tokio::time::Instant::now() + Duration::from_secs(16);
    while !pending_respawn_results.is_empty() {
        match tokio::time::timeout_at(cleanup_deadline, respawn_rx.recv()).await {
            Ok(Some(result)) => {
                pending_respawn_results.remove(&result.index);
                shutdown_respawn_results.push(result);
            }
            Ok(None) | Err(_) => {
                shutdown_verification.record(ShutdownOwner::Respawn, false);
                tracing::error!(
                    pending_slots = ?pending_respawn_results,
                    "timed out waiting for cancelled respawn ownership verification"
                );
                break;
            }
        }
    }
    while let Ok(result) = respawn_rx.try_recv() {
        shutdown_respawn_results.push(result);
    }

    // Explicitly shut down successfully returned agents instead of relying on
    // AcpClient::Drop.
    for rr in shutdown_respawn_results {
        match rr.result {
            Ok(Some((mut acp, _, _))) => {
                let verified =
                    shutdown_acp_with_log(&mut acp, Some(rr.index), "respawned agent shutdown")
                        .await;
                shutdown_verification.record(ShutdownOwner::Respawn, verified);
                if verified {
                    tracing::debug!(agent = rr.index, "reaped respawned agent on shutdown");
                }
            }
            Ok(None) => {}
            Err(_) => {
                if rr.failure_class == Some(RespawnFailureClass::CleanupUnverified) {
                    shutdown_verification.record(ShutdownOwner::Respawn, false);
                }
            }
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

    // Close the last process-level observer sender, then give the publisher a
    // bounded chance to drain every terminal result into the relay's
    // non-evicting priority state. A timeout or rejected admission is a
    // shutdown verification failure, never a silent proof loss.
    drop(observer);
    if let Some(handle) = relay_observer_publisher_task.take() {
        let abort_handle = handle.abort_handle();
        match tokio::time::timeout(Duration::from_secs(5), handle).await {
            Ok(Ok(true)) => {}
            Ok(Ok(false)) => {
                tracing::error!("terminal observer result delivery failed before shutdown");
                shutdown_verification.record(ShutdownOwner::ObserverPublisher, false);
            }
            Ok(Err(error)) => {
                tracing::error!("observer publisher task failed during shutdown: {error}");
                shutdown_verification.record(ShutdownOwner::ObserverPublisher, false);
            }
            Err(_) => {
                tracing::error!("observer publisher did not drain within 5s");
                abort_handle.abort();
                shutdown_verification.record(ShutdownOwner::ObserverPublisher, false);
            }
        }
    }

    // Graceful relay shutdown — sends WebSocket close frame and waits up to 5s
    // for the background task to finish, rather than aborting immediately (#40).
    relay.shutdown().await;

    shutdown_verification.into_result()?;
    tracing::info!("buzz-acp stopped");
    Ok(())
}

#[derive(PartialEq)]
enum LoopAction {
    Continue,
    Exit,
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
) -> bool {
    kind_u32 == KIND_STREAM_MESSAGE
        && event.content.trim() == command
        && event_mentions_agent(event, agent_pubkey_hex)
}

fn is_admitted_owner_control_command(
    event: &nostr::Event,
    kind_u32: u32,
    command: &str,
    agent_pubkey_hex: &str,
    privileged_control_admitted: bool,
) -> bool {
    privileged_control_admitted
        && is_owner_control_command(event, kind_u32, command, agent_pubkey_hex)
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ControlSignalResult {
    /// The task's control receiver accepted the signal.
    Delivered,
    /// A prior control already consumed (or closed) the receiver, but an
    /// explicit owner Cancel/Rotate disposition was recorded at the supervisor
    /// boundary and will dominate result/panic batch fate.
    DropRecorded,
    /// No matching task exists, or this non-drop control could not be sent.
    NotAccepted,
}

fn record_drop_control(meta: &mut pool::TaskMeta, control: AcceptedDropControl) {
    if meta.accepted_drop_control != Some(AcceptedDropControl::Rotate) {
        meta.accepted_drop_control = Some(control);
    }
}

/// Deliver a control signal or, for explicit Cancel/Rotate, persist its
/// stronger supervisor-owned drop disposition while the task still exists.
fn signal_in_flight_task(
    pool: &mut AgentPool,
    channel_id: uuid::Uuid,
    mode: ControlSignal,
) -> ControlSignalResult {
    let entry = pool
        .task_map_mut()
        .values_mut()
        .find(|m| m.channel_id == Some(channel_id));

    let Some(meta) = entry else {
        return ControlSignalResult::NotAccepted;
    };
    let accepted_drop_control = match &mode {
        ControlSignal::Cancel => Some(AcceptedDropControl::Cancel),
        ControlSignal::Rotate => Some(AcceptedDropControl::Rotate),
        ControlSignal::Steer | ControlSignal::Interrupt | ControlSignal::SwitchModel(_) => None,
    };
    let accepted_model_switch = match &mode {
        ControlSignal::SwitchModel(request) => Some(request.clone()),
        ControlSignal::Steer
        | ControlSignal::Interrupt
        | ControlSignal::Cancel
        | ControlSignal::Rotate => None,
    };

    let Some(tx) = meta.control_tx.take() else {
        if let Some(control) = accepted_drop_control {
            record_drop_control(meta, control);
            return ControlSignalResult::DropRecorded;
        }
        return ControlSignalResult::NotAccepted;
    };

    match tx.send(mode) {
        Ok(()) => {
            if let Some(control) = accepted_drop_control {
                record_drop_control(meta, control);
            }
            if let Some(request) = accepted_model_switch {
                meta.desired_model = Some(request.model_id.clone());
                meta.model_overridden = true;
                meta.accepted_model_switch = Some(request);
            }
            tracing::info!(
                channel = %channel_id,
                "control signal sent to in-flight task"
            );
            ControlSignalResult::Delivered
        }
        Err(mode) => {
            tracing::debug!(
                channel = %channel_id,
                ?mode,
                "in-flight task stopped accepting control signals"
            );
            if let Some(control) = accepted_drop_control {
                record_drop_control(meta, control);
                ControlSignalResult::DropRecorded
            } else {
                ControlSignalResult::NotAccepted
            }
        }
    }
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
    channel_id: uuid::Uuid,
    occurrence_id: EnqueueOccurrenceId,
    event: nostr::Event,
    prompt_tag: String,
    steer_ack_tx: &mpsc::UnboundedSender<SteerAckEvent>,
) -> bool {
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

    match pool.send_steer(channel_id, request) {
        Ok(()) => {
            // Withhold the queued event synchronously BEFORE spawning
            // the watcher: this closes the race where `mark_complete`
            // clears `in_flight_channels` and a stray `flush_next` could
            // re-deliver the event via normal dispatch. See
            // `EventQueue::mark_native_steer_pending` docs at queue.rs:606.
            let withheld = queue.mark_native_steer_pending(channel_id, occurrence_id);
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
                    occurrence_id,
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

fn defer_unpinned_batch_for_capacity(queue: &mut EventQueue, batch: FlushBatch) {
    let channel_id = batch.channel_id;
    queue.requeue_preserve_timestamps(batch);
    queue.mark_deferred(channel_id);
}

/// Flush queued work to available agents.
fn dispatch_pending(
    pool: &mut AgentPool,
    queue: &mut EventQueue,
    ctx: &Arc<PromptContext>,
) -> Vec<(Uuid, ThreadTags)> {
    let mut dispatched_channels = Vec::new();
    loop {
        let batch = match queue.flush_next() {
            Some(b) => b,
            None => break,
        };
        let channel_id = batch.channel_id;
        let typing_scope = batch
            .events
            .last()
            .map(|event| queue::parse_thread_tags(&event.event))
            .unwrap_or_default();
        let required_agent = queue.required_agent(channel_id);
        let affinity_hit = required_agent.is_some() || pool.has_session_for(channel_id);
        let mut agent = match required_agent
            .and_then(|index| pool.try_claim_index(index))
            .or_else(|| {
                if required_agent.is_none() {
                    pool.try_claim(Some(channel_id))
                } else {
                    None
                }
            }) {
            Some(a) => a,
            None => {
                if let Some(required_agent) = required_agent {
                    tracing::debug!(
                        channel = %channel_id,
                        agent = required_agent,
                        "required agent slot unavailable — deferring pinned batch"
                    );
                    queue.requeue_preserve_timestamps(batch);
                    queue.block_required_agent(channel_id);
                    queue.mark_deferred(channel_id);
                    continue;
                }
                let pending = queue.pending_channels();
                tracing::debug!(pending_channels = pending, "pool_exhausted");
                defer_unpinned_batch_for_capacity(queue, batch);
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
        let desired_model = agent.desired_model.clone();
        let model_overridden = agent.model_overridden;

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
                turn_id,
                recoverable_batch,
                desired_model,
                model_overridden,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: Some(control_tx),
                steer_tx,
            },
        );
        dispatched_channels.push((channel_id, typing_scope));
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

fn acp_error_class(error: &acp::AcpError) -> &'static str {
    match error {
        acp::AcpError::Io(_) => "io",
        acp::AcpError::Json(_) => "json",
        acp::AcpError::AgentExited => "agent_exited",
        acp::AcpError::IdleTimeout(_) => "idle_timeout",
        acp::AcpError::HardTimeout { .. } => "hard_timeout",
        acp::AcpError::CancelDrainTimeout(_) => "cancel_drain_timeout",
        acp::AcpError::Timeout(_) => "timeout",
        acp::AcpError::WriteTimeout(_) => "write_timeout",
        acp::AcpError::Protocol(_) => "protocol",
        acp::AcpError::AgentError { .. } => "agent_error",
    }
}

fn outcome_replaces_adapter(outcome: &PromptOutcome) -> bool {
    matches!(
        outcome,
        PromptOutcome::AgentExited
            | PromptOutcome::Timeout(_)
            | PromptOutcome::CancelDrainTimeout(_)
            | PromptOutcome::CancelCleanupFailed(_)
            | PromptOutcome::SessionCloseFailed(_)
            | PromptOutcome::SessionRecycleRequired
            | PromptOutcome::SessionSetupCloseFailed(_)
            | PromptOutcome::SessionSetupRecycleRequired
            | PromptOutcome::ControlPreemptedSetup
    ) || matches!(
        outcome,
        PromptOutcome::Error(error) if error.requires_process_replacement()
    )
}

fn outcome_releases_source_session(outcome: &PromptOutcome) -> bool {
    outcome_replaces_adapter(outcome)
        || matches!(
            outcome,
            PromptOutcome::SessionRetired(_) | PromptOutcome::Cancelled
        )
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
    rest_client: Option<&relay::RestClient>,
) -> LoopAction {
    let before = pool.task_map().len();
    let agent_index = result.agent.index;
    let completed_task_id = pool
        .task_map()
        .iter()
        .find_map(|(task_id, meta)| (meta.agent_index == agent_index).then_some(*task_id));
    let completed_meta = completed_task_id.and_then(|task_id| pool.task_map_mut().remove(&task_id));
    debug_assert_eq!(before, pool.task_map().len() + 1);
    let accepted_drop_control = completed_meta
        .as_ref()
        .and_then(|meta| meta.accepted_drop_control);
    let mut recycle_for_unconsumed_control = completed_meta
        .as_ref()
        .is_some_and(|meta| meta.accepted_drop_control == Some(AcceptedDropControl::Rotate))
        && !outcome_releases_source_session(&result.outcome);
    if result.retry_agent_index.is_none() {
        if let Some(request) = completed_meta
            .as_ref()
            .and_then(|meta| meta.accepted_model_switch.as_ref())
        {
            // The main loop successfully delivered SwitchModel, but the prompt
            // future may have won the task's biased simultaneous-ready race.
            // Reuse the task-side catalog gate here: this returned agent still
            // owns both the prior intent and the capabilities that accepted or
            // reject the request. Only an accepted model may pin the replay and
            // recycle the still-owning adapter.
            if pool::stage_busy_model_switch_intent(&mut result.agent, request) {
                result.retry_agent_index = Some(agent_index);
                recycle_for_unconsumed_control |= !outcome_releases_source_session(&result.outcome);
                tracing::info!(
                    agent = agent_index,
                    model = %request.model_id,
                    "reconciled acknowledged model switch after prompt/control ready race"
                );
            } else {
                tracing::warn!(
                    agent = agent_index,
                    model = %request.model_id,
                    "rejected unsupported model switch after prompt/control ready race"
                );
            }
        }
    }
    let removed_session_requires_recycle = removed_channels
        .iter()
        .any(|channel_id| result.agent.state.sessions.contains_key(channel_id))
        && !outcome_replaces_adapter(&result.outcome);
    let lifecycle_recycle_required =
        recycle_for_unconsumed_control || removed_session_requires_recycle;
    if lifecycle_recycle_required && result.agent.model_overridden {
        result.retry_agent_index.get_or_insert(agent_index);
    }
    let result_channel = match &result.source {
        PromptSource::Channel(channel_id) => Some(*channel_id),
        PromptSource::Heartbeat => None,
    };
    let existing_required_agent =
        result_channel.and_then(|channel_id| queue.required_agent(channel_id));
    let replacement_preserves_override = result.agent.model_switch_request_id.is_some()
        || (result.agent.model_overridden
            && matches!(
                result.outcome,
                PromptOutcome::SessionRetired(_)
                    | PromptOutcome::Cancelled
                    | PromptOutcome::CancelDrainTimeout(_)
                    | PromptOutcome::CancelCleanupFailed(_)
                    | PromptOutcome::SessionCloseFailed(_)
                    | PromptOutcome::SessionRecycleRequired
                    | PromptOutcome::SessionSetupCloseFailed(_)
                    | PromptOutcome::SessionSetupRecycleRequired
                    | PromptOutcome::ControlPreemptedSetup
            ));
    let established_required_agent = result
        .retry_agent_index
        .or(replacement_preserves_override.then_some(agent_index));
    let preserve_model_on_fatal_replacement =
        established_required_agent.is_some() || existing_required_agent == Some(agent_index);
    if let (Some(channel_id), Some(required_agent)) = (result_channel, established_required_agent) {
        if !removed_channels.contains(&channel_id) {
            queue.require_agent(channel_id, required_agent);
        }
    }

    // The hard-timeout death_message (below) must describe the batch's
    // *actual* fate, not just the `recently_active` eligibility flag — a
    // recently-active batch that exhausts the retry budget in queue.requeue()
    // is dead-lettered same as an immediate one, and both differ from a
    // channel-removed drop or a heartbeat call with no batch at all. Each
    // branch below records what actually happened; only the hard-timeout
    // match arm in the death_message construction reads it.
    let mut hard_timeout_fate_suffix: Option<&'static str> = None;
    let mut batch_requeued = false;
    let mut batch_terminally_disposed = false;

    // Requeue BEFORE mark_complete: requeue() sets retry_after with a future
    // deadline, and mark_complete() checks for it to decide whether to preserve
    // retry_counts. If mark_complete runs first, retry_counts is cleared and
    // every retry starts at attempt 1 — defeating exponential backoff and
    // dead-letter protection.
    if let Some(batch) = result.batch.take() {
        if let Some(control) = accepted_drop_control {
            tracing::info!(
                channel_id = %batch.channel_id,
                events = batch.events.len(),
                ?control,
                "dropping batch after acknowledged owner control"
            );
            hard_timeout_fate_suffix = Some(" — batch dropped (owner control)");
            batch_terminally_disposed = true;
        // Don't requeue batches for channels the agent was removed from —
        // those events are stale and should be silently dropped.
        } else if !removed_channels.contains(&batch.channel_id) {
            if matches!(
                result.outcome,
                PromptOutcome::Cancelled
                    | PromptOutcome::CancelDrainTimeout(_)
                    | PromptOutcome::CancelCleanupFailed(_)
                    | PromptOutcome::SessionCloseFailed(_)
                    | PromptOutcome::SessionRecycleRequired
                    | PromptOutcome::ControlPreemptedSetup
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
                // CancelDrainTimeout and SessionCloseFailed share this path
                // with Cancelled: cleanup did not complete, but that is not
                // the deterministic hard-cap death below — the original batch
                // must survive with no retry/dead-letter accounting, same as
                // a clean cancel.
                let reason = batch.cancel_reason.unwrap_or(CancelReason::Steer);
                queue.requeue_as_cancelled(batch, reason);
                batch_requeued = true;
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
                batch_terminally_disposed = true;
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
                    batch_terminally_disposed = true;
                } else {
                    hard_timeout_fate_suffix = Some(" — requeued for retry (recently active)");
                    batch_requeued = true;
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
                batch_terminally_disposed = true;
            } else {
                match queue.requeue(batch) {
                    Some(dead) => {
                        let reason = match &result.outcome {
                            PromptOutcome::Timeout(TimeoutKind::Idle) => {
                                "the turn timed out".to_string()
                            }
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
                        batch_terminally_disposed = true;
                    }
                    None => batch_requeued = true,
                }
            }
        } else {
            tracing::debug!(
                channel_id = %batch.channel_id,
                events = batch.events.len(),
                "dropping failed batch for removed channel"
            );
            hard_timeout_fate_suffix = Some(" — batch dropped (channel removed)");
            batch_terminally_disposed = true;
        }
    }

    match &result.source {
        PromptSource::Channel(ch) => {
            queue.mark_complete(*ch);
            let replacement_resets_model = match &result.outcome {
                PromptOutcome::AgentExited | PromptOutcome::Timeout(_) => true,
                PromptOutcome::Error(error) => error.requires_process_replacement(),
                _ => false,
            };
            let keep_required_agent = !batch_terminally_disposed
                && (established_required_agent.is_some()
                    || (replacement_resets_model && preserve_model_on_fatal_replacement)
                    || (batch_requeued && !replacement_resets_model));
            if !keep_required_agent {
                if batch_terminally_disposed {
                    queue.clear_required_agent_if_drained(*ch);
                } else {
                    queue.clear_required_agent(*ch);
                }
            }
        }
        PromptSource::Heartbeat => *heartbeat_in_flight = false,
    }

    if lifecycle_recycle_required {
        // Batch fate and exact-slot bookkeeping above intentionally used the
        // original prompt outcome. Only now replace the healthy adapter, so an
        // acknowledged rotate/model switch or membership removal can never
        // abandon a remote session via local map deletion.
        result.outcome = PromptOutcome::SessionRecycleRequired;
    }

    let outcome_label = match &result.outcome {
        PromptOutcome::Ok(_) => "ok",
        PromptOutcome::SessionRetired(_) => "session_retired",
        PromptOutcome::Error(_) => "error",
        PromptOutcome::Timeout(TimeoutKind::Idle) => "idle_timeout",
        PromptOutcome::Timeout(TimeoutKind::Hard { .. }) => "hard_timeout",
        PromptOutcome::AgentExited => "exited",
        PromptOutcome::Cancelled => "cancelled",
        PromptOutcome::CancelDrainTimeout(_) => "cancel_drain_timeout",
        PromptOutcome::CancelCleanupFailed(_) => "cancel_cleanup_failed",
        PromptOutcome::SessionCloseFailed(_) => "session_close_failed",
        PromptOutcome::SessionRecycleRequired => "session_recycle_required",
        PromptOutcome::SessionSetupCloseFailed(_) => "session_setup_close_failed",
        PromptOutcome::SessionSetupRecycleRequired => "session_setup_recycle_required",
        PromptOutcome::ControlPreemptedSetup => "control_preempted_setup",
    };
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
        PromptOutcome::Ok(_) | PromptOutcome::SessionRetired(_) => {
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
            let spawned = if preserve_model_on_fatal_replacement {
                spawn_respawn_task_preserving_model_intent(
                    result.agent,
                    config,
                    slot_history,
                    respawn_tx,
                    respawn_tasks,
                    observer.clone(),
                )
            } else {
                spawn_respawn_task(
                    result.agent,
                    config,
                    slot_history,
                    respawn_tx,
                    respawn_tasks,
                    observer.clone(),
                )
            };
            if !spawned {
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
            if !spawn_respawn_task_preserving_model_intent(
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
        PromptOutcome::CancelCleanupFailed(error) => {
            let error_class = acp_error_class(&error);
            let error_code = match &error {
                acp::AcpError::AgentError { code, .. } => Some(*code),
                _ => None,
            };
            tracing::warn!(
                agent = agent_index,
                outcome = outcome_label,
                configured_model = %harness_configured_model,
                pid = harness_pid,
                error_class,
                error_code = ?error_code,
                "agent_returned — respawning (cancel cleanup failed; adapter-controlled details redacted)"
            );
            let death_message = format!(
                "Agent cancellation cleanup failed ({error_class}); the agent process is being replaced."
            );
            emit_turn_error(&death_message, error_code);

            let index = result.agent.index;
            let slot_history = &mut crash_history[index];
            if !spawn_respawn_task_preserving_model_intent(
                result.agent,
                config,
                slot_history,
                respawn_tx,
                respawn_tasks,
                observer.clone(),
            ) && pool.live_count() == 0
                && !any_respawn_in_flight(crash_history)
            {
                tracing::error!("all agents dead — exiting");
                return LoopAction::Exit;
            }
        }
        // `session/close` is optional ACP. A healthy adapter that does not
        // advertise it cannot safely be reused after local retirement because
        // only process teardown can release its session tree. Replace it
        // immediately without charging crash budget or losing a live model
        // override.
        PromptOutcome::SessionRecycleRequired
        | PromptOutcome::SessionSetupRecycleRequired
        | PromptOutcome::ControlPreemptedSetup => {
            tracing::info!(
                agent = agent_index,
                outcome = outcome_label,
                configured_model = %harness_configured_model,
                pid = harness_pid,
                "agent_returned — recycling (session close unsupported)"
            );
            let index = result.agent.index;
            let slot_history = &mut crash_history[index];
            let switch_control = match (
                &result.source,
                result.agent.desired_model.as_ref(),
                result.agent.model_switch_request_id.as_ref(),
            ) {
                (PromptSource::Channel(channel_id), Some(model_id), Some(request_id)) => {
                    Some(PendingSwitchControl {
                        channel_id: *channel_id,
                        model_id: model_id.clone(),
                        request_id: request_id.clone(),
                    })
                }
                _ => None,
            };
            if !spawn_recycle_task_with_switch(
                result.agent,
                config,
                slot_history,
                respawn_tx,
                respawn_tasks,
                observer.clone(),
                switch_control,
            ) && pool.live_count() == 0
                && !any_respawn_in_flight(crash_history)
            {
                tracing::error!("all agents dead — exiting");
                return LoopAction::Exit;
            }
        }
        // Session retirement is fail-closed: once `session/close` fails, Buzz
        // no longer knows whether the adapter still owns the session's query
        // tree. Error class is irrelevant — even method-not-found is unsafe to
        // reuse because local invalidation would abandon those resources.
        PromptOutcome::SessionCloseFailed(error)
        | PromptOutcome::SessionSetupCloseFailed(error) => {
            let error_class = acp_error_class(&error);
            let error_code = match &error {
                acp::AcpError::AgentError { code, .. } => Some(*code),
                _ => None,
            };
            tracing::warn!(
                agent = agent_index,
                outcome = outcome_label,
                configured_model = %harness_configured_model,
                pid = harness_pid,
                error_class,
                error_code = ?error_code,
                "agent_returned — respawning (session close failed; adapter-controlled details redacted)"
            );
            let death_message = format!(
                "Agent session cleanup failed ({error_class}); the agent process is being replaced."
            );
            emit_turn_error(&death_message, error_code);

            let index = result.agent.index;
            let slot_history = &mut crash_history[index];
            if !spawn_respawn_task_preserving_model_intent(
                result.agent,
                config,
                slot_history,
                respawn_tx,
                respawn_tasks,
                observer.clone(),
            ) && pool.live_count() == 0
                && !any_respawn_in_flight(crash_history)
            {
                tracing::error!("all agents dead — exiting");
                return LoopAction::Exit;
            }
        }
        // Errors fall into two categories:
        //
        // 1. Transport/framing-class (Io, WriteTimeout, Timeout, Protocol,
        //    Json): the stdio
        //    pipe may be corrupted or the agent desynchronized. These are fatal
        //    to the agent regardless of whether they occurred during session
        //    creation or an active prompt — respawn unconditionally.
        //
        // 2. Application-class (agent JSON-RPC errors): the pipe is
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
            let is_transport_error = e.requires_process_replacement();
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
                let spawned = if preserve_model_on_fatal_replacement {
                    spawn_respawn_task_preserving_model_intent(
                        result.agent,
                        config,
                        slot_history,
                        respawn_tx,
                        respawn_tasks,
                        observer,
                    )
                } else {
                    spawn_respawn_task(
                        result.agent,
                        config,
                        slot_history,
                        respawn_tx,
                        respawn_tasks,
                        observer,
                    )
                };
                if !spawned && pool.live_count() == 0 && !any_respawn_in_flight(crash_history) {
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
    if pool.slot_alive(agent_index) {
        queue.release_required_agent(agent_index);
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
    typing_channels: &mut HashMap<Uuid, ThreadTags>,
    crash_history: &mut [SlotCircuit],
    _respawn_tx: &mpsc::Sender<RespawnResult>,
    _respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<observer::ObserverHandle>,
    rest_client: Option<&relay::RestClient>,
) {
    let task_id = join_error.id();
    let Some(meta) = pool.task_map_mut().remove(&task_id) else {
        tracing::error!("panic for unknown task {task_id:?} — bug");
        return;
    };
    let i = meta.agent_index;
    let accepted_drop_control = meta.accepted_drop_control;
    let preserve_model_intent = meta.model_overridden
        || meta
            .channel_id
            .is_some_and(|channel_id| queue.required_agent(channel_id) == Some(i));
    if preserve_model_intent {
        if let Some(channel_id) = meta.channel_id {
            if !removed_channels.contains(&channel_id) {
                queue.require_agent(channel_id, i);
            }
        }
    }

    // Requeue BEFORE mark_complete (same rationale as handle_prompt_result).
    if let Some(batch) = meta.recoverable_batch {
        if let Some(ch) = meta.channel_id {
            if let Some(control) = accepted_drop_control {
                tracing::info!(
                    agent = i,
                    channel = %ch,
                    events = batch.events.len(),
                    ?control,
                    "dropping panicked batch after acknowledged owner control"
                );
            } else if !removed_channels.contains(&ch) {
                match queue.requeue(batch) {
                    Some(dead) => {
                        queue.clear_required_agent_if_drained(ch);
                        tracing::error!(
                            agent = i,
                            channel = %ch,
                            events = dead.events.len(),
                            "dead-lettered batch after repeated panics"
                        );
                        spawn_failure_notice(
                            rest_client,
                            &dead,
                            "⚠️ I couldn't process the last request after repeated internal task failures. Please re-send if it's still needed."
                                .to_string(),
                        );
                    }
                    None => tracing::warn!("requeued batch for panicked agent {i}"),
                }
            } else {
                tracing::debug!(
                    channel_id = %ch,
                    "dropping panicked batch for removed channel"
                );
            }
        }
    }

    if let Some(ch) = meta.channel_id {
        queue.mark_complete(ch);
        typing_channels.remove(&ch);
        tracing::warn!("cleared wedged in-flight channel {ch} from panicked agent {i}");
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
                "slotQuarantined": true,
            }),
        );
    }

    // Unwinding destroyed the only AcpClient/Child handle. Drop issued a
    // best-effort group signal, but it cannot await direct-child reaping or
    // prove process-group absence. Never translate that ambiguity into a timed
    // respawn: quarantine the slot for this supervisor process so maintenance
    // and crash cooldowns cannot create an overlapping adapter.
    let slot = &mut crash_history[i];
    let (desired_model, model_overridden) = if preserve_model_intent {
        (meta.desired_model.clone(), meta.model_overridden)
    } else {
        (config.model.clone(), false)
    };
    slot.pending_model_intent = preserve_model_intent.then_some(ReplacementModelIntent {
        desired_model,
        model_overridden,
        model_switch_request_id: meta
            .accepted_model_switch
            .as_ref()
            .map(|request| request.request_id.clone()),
        model_switch_rollback: None,
    });
    slot.mark_cleanup_unverified();
    tracing::error!(
        agent = i,
        "panicked adapter cleanup cannot be verified — slot quarantined until process restart"
    );
}

#[allow(clippy::too_many_arguments)]
fn drain_ready_join_results(
    pool: &mut AgentPool,
    queue: &mut EventQueue,
    config: &Config,
    heartbeat_in_flight: &mut bool,
    removed_channels: &HashSet<Uuid>,
    typing_channels: &mut HashMap<Uuid, ThreadTags>,
    crash_history: &mut [SlotCircuit],
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<observer::ObserverHandle>,
    rest_client: Option<&relay::RestClient>,
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
                rest_client,
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
    let desired_model = agent.desired_model.clone();
    let model_overridden = agent.model_overridden;
    let turn_id = Uuid::new_v4().to_string();
    let task_turn_id = turn_id.clone();
    let (control_tx, control_rx) = tokio::sync::oneshot::channel::<ControlSignal>();

    let abort_handle = pool.join_set.spawn(async move {
        pool::run_prompt_task(
            agent,
            None,
            Some(prompt_text),
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
            channel_id: None,
            turn_id,
            recoverable_batch: None,
            desired_model,
            model_overridden,
            accepted_model_switch: None,
            accepted_drop_control: None,
            control_tx: Some(control_tx),
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
    spawn_respawn_task_with_policy(
        old_agent,
        config,
        slot,
        respawn_tx,
        respawn_tasks,
        observer,
        false,
    )
}

/// Cleanup-triggered crash replacement preserves the live model intent until
/// a replacement initializes. This includes failed session retirement and
/// bounded cancel cleanup; neither is an operator-requested model reset.
fn spawn_respawn_task_preserving_model_intent(
    old_agent: OwnedAgent,
    config: &Config,
    slot: &mut SlotCircuit,
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<observer::ObserverHandle>,
) -> bool {
    spawn_respawn_task_with_policy(
        old_agent,
        config,
        slot,
        respawn_tx,
        respawn_tasks,
        observer,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_respawn_task_with_policy(
    old_agent: OwnedAgent,
    config: &Config,
    slot: &mut SlotCircuit,
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<observer::ObserverHandle>,
    retain_model_intent: bool,
) -> bool {
    let index = old_agent.index;
    let (desired_model, model_overridden) = if retain_model_intent {
        (old_agent.desired_model.clone(), old_agent.model_overridden)
    } else {
        (config.model.clone(), false)
    };

    // Circuit breaker: record crash, decide whether to respawn.
    let delay = match slot.record_crash() {
        CrashVerdict::CircuitOpen => {
            tracing::error!(
                agent = index,
                "circuit open — suppressing replacement after bounded shutdown"
            );
            return spawn_cleanup_only_task(
                old_agent,
                slot,
                respawn_tx,
                respawn_tasks,
                desired_model,
                model_overridden,
                retain_model_intent,
            );
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

    spawn_replacement_task(
        old_agent,
        config,
        slot,
        respawn_tx,
        respawn_tasks,
        observer,
        delay,
        desired_model,
        model_overridden,
        retain_model_intent,
        None,
    )
}

/// Replace a healthy adapter whose negotiated capabilities cannot explicitly
/// retire sessions. This is lifecycle maintenance, not crash recovery: it
/// bypasses crash accounting and preserves any live model override.
fn spawn_recycle_task(
    old_agent: OwnedAgent,
    config: &Config,
    slot: &mut SlotCircuit,
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<observer::ObserverHandle>,
) -> bool {
    spawn_recycle_task_with_switch(
        old_agent,
        config,
        slot,
        respawn_tx,
        respawn_tasks,
        observer,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_model_switch_recycle_task(
    old_agent: OwnedAgent,
    config: &Config,
    slot: &mut SlotCircuit,
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<observer::ObserverHandle>,
    control: PendingSwitchControl,
) -> bool {
    spawn_recycle_task_with_switch(
        old_agent,
        config,
        slot,
        respawn_tx,
        respawn_tasks,
        observer,
        Some(control),
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_recycle_task_with_switch(
    old_agent: OwnedAgent,
    config: &Config,
    slot: &mut SlotCircuit,
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<observer::ObserverHandle>,
    switch_control: Option<PendingSwitchControl>,
) -> bool {
    let desired_model = old_agent.desired_model.clone();
    let model_overridden = old_agent.model_overridden;
    let delay = slot.schedule_recycle();
    spawn_replacement_task(
        old_agent,
        config,
        slot,
        respawn_tx,
        respawn_tasks,
        observer,
        delay,
        desired_model,
        model_overridden,
        true,
        switch_control,
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_cleanup_only_task(
    old_agent: OwnedAgent,
    slot: &mut SlotCircuit,
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    desired_model: Option<String>,
    model_overridden: bool,
    retain_model_intent: bool,
) -> bool {
    let index = old_agent.index;
    debug_assert!(
        !slot.respawn_in_flight,
        "an owned completing agent cannot already have cleanup in flight"
    );
    slot.respawn_in_flight = true;
    slot.pending_model_intent = retain_model_intent.then_some(ReplacementModelIntent {
        desired_model: desired_model.clone(),
        model_overridden,
        model_switch_request_id: if retain_model_intent {
            old_agent.model_switch_request_id.clone()
        } else {
            None
        },
        model_switch_rollback: if retain_model_intent {
            old_agent.model_switch_rollback.clone()
        } else {
            None
        },
    });
    let guard = RespawnGuard::new(
        index,
        respawn_tx.clone(),
        desired_model,
        model_overridden,
        retain_model_intent,
    );
    respawn_tasks.spawn(async move {
        let mut agent = old_agent;
        if let Err(error) = agent.acp.shutdown().await {
            tracing::error!(
                agent = index,
                error = %error,
                "old adapter shutdown could not be verified; circuit remains open"
            );
            guard.send_cleanup_unverified(anyhow::anyhow!(
                "old adapter cleanup unverified: {error}"
            ));
            return;
        }
        drop(agent);
        guard.send_cleanup_complete();
    });
    true
}

#[allow(clippy::too_many_arguments)]
fn spawn_replacement_task(
    old_agent: OwnedAgent,
    config: &Config,
    slot: &mut SlotCircuit,
    respawn_tx: &mpsc::Sender<RespawnResult>,
    respawn_tasks: &mut tokio::task::JoinSet<()>,
    observer: Option<observer::ObserverHandle>,
    delay: Duration,
    desired_model: Option<String>,
    model_overridden: bool,
    retain_model_intent: bool,
    switch_control: Option<PendingSwitchControl>,
) -> bool {
    let index = old_agent.index;
    debug_assert!(
        !slot.respawn_in_flight,
        "an owned completing agent cannot already have a replacement in flight"
    );
    slot.respawn_in_flight = true;
    slot.pending_model_intent = retain_model_intent.then_some(ReplacementModelIntent {
        desired_model: desired_model.clone(),
        model_overridden,
        model_switch_request_id: if retain_model_intent {
            old_agent.model_switch_request_id.clone()
        } else {
            None
        },
        model_switch_rollback: if retain_model_intent {
            old_agent.model_switch_rollback.clone()
        } else {
            None
        },
    });

    // Spawn the actual work (shutdown + sleep + spawn + init) off the main loop.
    let cmd = config.agent_command.clone();
    let args = config.agent_args.clone();
    let env = config.persona_env_vars.clone();
    let has_codex = config.has_generated_codex_config;
    let mut guard = RespawnGuard::new(
        index,
        respawn_tx.clone(),
        desired_model,
        model_overridden,
        retain_model_intent,
    )
    .with_switch_failure(observer.clone(), switch_control);
    respawn_tasks.spawn(async move {
        // Shutdown old agent (reap child, prevent zombie).
        let mut agent = old_agent;
        if let Err(error) = agent.acp.shutdown().await {
            tracing::error!(
                agent = index,
                error = %error,
                "old adapter shutdown could not be verified; refusing replacement spawn"
            );
            guard.send_cleanup_unverified(anyhow::anyhow!(
                "old adapter cleanup unverified: {error}"
            ));
            return;
        }
        drop(agent);
        guard.mark_owner_cleanup_verified();

        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }

        let result =
            spawn_and_init(&cmd, &args, &env, has_codex, index, observer, &mut guard).await;
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

async fn shutdown_acp_with_log(
    acp: &mut AcpClient,
    agent_index: Option<usize>,
    phase: &'static str,
) -> bool {
    match acp.shutdown().await {
        Ok(()) => true,
        Err(error) => {
            tracing::error!(
                agent = agent_index,
                phase,
                error = %error,
                "ACP adapter shutdown could not be verified"
            );
            false
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownOwner {
    Wake,
    CheckedOut,
    Respawn,
    Idle,
    ObserverPublisher,
}

impl ShutdownOwner {
    fn label(self) -> &'static str {
        match self {
            Self::Wake => "pool initialization",
            Self::CheckedOut => "checked-out agent",
            Self::Respawn => "respawn agent",
            Self::Idle => "idle agent",
            Self::ObserverPublisher => "observer terminal-result publisher",
        }
    }
}

#[derive(Default)]
struct ShutdownVerification {
    unverified: Vec<ShutdownOwner>,
}

impl ShutdownVerification {
    fn record(&mut self, owner: ShutdownOwner, verified: bool) {
        if !verified && !self.unverified.contains(&owner) {
            self.unverified.push(owner);
        }
    }

    fn into_result(self) -> Result<()> {
        if self.unverified.is_empty() {
            return Ok(());
        }
        let owners = self
            .unverified
            .into_iter()
            .map(ShutdownOwner::label)
            .collect::<Vec<_>>()
            .join(", ");
        Err(anyhow::anyhow!(
            "ACP shutdown ownership could not be verified for: {owners}"
        ))
    }
}

async fn shutdown_agent_slots(slots: &mut [Option<OwnedAgent>]) -> bool {
    let mut cleanup_verified = true;
    for slot in slots {
        if let Some(mut agent) = slot.take() {
            if !shutdown_acp_with_log(
                &mut agent.acp,
                Some(agent.index),
                "pool initialization cleanup",
            )
            .await
            {
                cleanup_verified = false;
            }
        }
    }
    cleanup_verified
}

fn cleanup_ownership_unverified(
    pool_cleanup_unverified: bool,
    crash_history: &[SlotCircuit],
) -> bool {
    pool_cleanup_unverified
        || crash_history
            .iter()
            .any(SlotCircuit::blocks_supervisor_exit)
}

async fn await_automatic_exit_permission(
    shutdown: &mut watch::Receiver<()>,
    cleanup_unverified: bool,
    reason: &'static str,
) {
    if !cleanup_unverified {
        return;
    }
    tracing::error!(
        reason,
        "automatic loop exit blocked by unverified adapter ownership; \
         holding non-spawning quarantine until explicit shutdown"
    );
    let _ = shutdown.changed().await;
}

async fn shutdown_agent_pool(pool: &mut AgentPool) -> bool {
    let mut cleanup_verified = true;
    let signalled = pool.cancel_checked_out_for_shutdown();
    tracing::debug!(signalled, "cooperatively cancelling checked-out agents");
    while let Some(result) = pool.join_set.join_next().await {
        if let Err(error) = result {
            tracing::error!(error = %error, "pool task failed during shutdown cleanup");
            cleanup_verified = false;
        }
    }
    while let Ok(mut result) = pool.result_rx_try_recv() {
        if !shutdown_acp_with_log(
            &mut result.agent.acp,
            Some(result.agent.index),
            "pool result cleanup",
        )
        .await
        {
            cleanup_verified = false;
        }
    }
    for slot in pool.agents_mut() {
        if let Some(mut agent) = slot.take() {
            if !shutdown_acp_with_log(&mut agent.acp, Some(agent.index), "pool slot cleanup").await
            {
                cleanup_verified = false;
            }
        }
    }
    cleanup_verified
}

async fn fail_pre_loop_with_pool_cleanup<T>(
    pool: &mut AgentPool,
    shutdown: &mut watch::Receiver<()>,
    phase: &'static str,
    error: anyhow::Error,
    cleanup_already_unverified: bool,
    shutdown_requested: bool,
) -> Result<T> {
    let cleanup_unverified = cleanup_already_unverified || !shutdown_agent_pool(pool).await;
    if cleanup_unverified && !shutdown_requested {
        // An automatic pre-loop failure must not let a service manager restart
        // this process while any previously spawned adapter may still exist.
        // Remain in the non-spawning quarantine until an explicit signal
        // requests process shutdown.
        tracing::error!(
            phase,
            error = %error,
            "pre-loop failure with unverified adapter cleanup — holding quarantine until shutdown"
        );
        let _ = shutdown.changed().await;
    }

    if cleanup_unverified {
        Err(anyhow::anyhow!(
            "{phase}: {error}; adapter cleanup remains unverified"
        ))
    } else {
        Err(anyhow::anyhow!("{phase}: {error}"))
    }
}

async fn await_pre_loop_operation<T, E, F>(
    operation: F,
    shutdown: &mut watch::Receiver<()>,
    pool: &mut AgentPool,
    phase: &'static str,
    cleanup_already_unverified: bool,
) -> Result<T>
where
    E: std::fmt::Display,
    F: std::future::Future<Output = std::result::Result<T, E>>,
{
    tokio::select! {
        biased;
        _ = shutdown.changed() => {
            fail_pre_loop_with_pool_cleanup(
                pool,
                shutdown,
                phase,
                anyhow::anyhow!("shutdown signal received before the main loop"),
                cleanup_already_unverified,
                true,
            )
            .await
        }
        result = operation => match result {
            Ok(value) => Ok(value),
            Err(error) => {
                fail_pre_loop_with_pool_cleanup(
                    pool,
                    shutdown,
                    phase,
                    anyhow::anyhow!("{error}"),
                    cleanup_already_unverified,
                    false,
                )
                .await
            }
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

#[derive(Debug, thiserror::Error)]
enum PoolStartupFailure {
    #[error("{0}")]
    Retryable(String),
    #[error("{0}")]
    CleanupUnverified(String),
}

enum InitialPoolStartup {
    Ready(AgentPool),
    Dormant,
    CleanupQuarantine(String),
}

fn resolve_initial_pool_startup(
    lazy_pool: bool,
    eager_result: Option<std::result::Result<AgentPool, PoolStartupFailure>>,
) -> std::result::Result<InitialPoolStartup, PoolStartupFailure> {
    if lazy_pool {
        debug_assert!(eager_result.is_none());
        return Ok(InitialPoolStartup::Dormant);
    }

    match eager_result.expect("eager startup must provide an initialization result") {
        Ok(pool) => Ok(InitialPoolStartup::Ready(pool)),
        Err(PoolStartupFailure::CleanupUnverified(error)) => {
            Ok(InitialPoolStartup::CleanupQuarantine(error))
        }
        Err(error) => Err(error),
    }
}

fn lazy_pool_can_wake(lazy_pool: bool, pool_ready: bool, cleanup_unverified: bool) -> bool {
    lazy_pool && !pool_ready && !cleanup_unverified
}

impl PoolStartupFailure {
    fn cleanup_unverified(&self) -> bool {
        matches!(self, Self::CleanupUnverified(_))
    }
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
) -> std::result::Result<AgentPool, PoolStartupFailure> {
    // One agent failing to start need not kill the whole pool after its child
    // is proved absent. Attempt each initialization under a 60-second timeout;
    // a partial pool is valid only across verified cleanup boundaries.
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
                            let current_cleanup_verified = shutdown_acp_with_log(
                                &mut acp,
                                Some(i),
                                "cancelled pool initialization",
                            )
                            .await;
                            let owned_cleanup_verified =
                                shutdown_agent_slots(&mut agent_slots).await;
                            if !current_cleanup_verified || !owned_cleanup_verified {
                                return Err(PoolStartupFailure::CleanupUnverified(format!(
                                    "pool initialization cancelled by shutdown; adapter cleanup \
                                     unverified at or before slot {i}"
                                )));
                            }
                            return Err(PoolStartupFailure::Retryable(
                                "pool initialization cancelled by shutdown".into(),
                            ));
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
                            model_switch_request_id: None,
                            model_switch_rollback: None,
                            agent_name,
                            goose_system_prompt_supported: None,
                            protocol_version,
                        }));
                    }
                    Ok(Err(e)) => {
                        tracing::error!(agent = i, "agent initialize failed: {e}");
                        if !shutdown_acp_with_log(&mut acp, Some(i), "failed pool initialization")
                            .await
                        {
                            let owned_cleanup_verified =
                                shutdown_agent_slots(&mut agent_slots).await;
                            return Err(PoolStartupFailure::CleanupUnverified(format!(
                                "agent {i} initialize failed: {e}; spawned adapter cleanup \
                                 unverified{}",
                                if owned_cleanup_verified {
                                    ""
                                } else {
                                    "; cleanup of an already-initialized adapter was also unverified"
                                }
                            )));
                        }
                        agent_slots.push(None);
                    }
                    Err(_) => {
                        tracing::error!(agent = i, "agent timed out during init (60s)");
                        if !shutdown_acp_with_log(
                            &mut acp,
                            Some(i),
                            "timed-out pool initialization",
                        )
                        .await
                        {
                            let owned_cleanup_verified =
                                shutdown_agent_slots(&mut agent_slots).await;
                            return Err(PoolStartupFailure::CleanupUnverified(format!(
                                "agent {i} timed out during initialization; spawned adapter \
                                 cleanup unverified{}",
                                if owned_cleanup_verified {
                                    ""
                                } else {
                                    "; cleanup of an already-initialized adapter was also unverified"
                                }
                            )));
                        }
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
        return Err(PoolStartupFailure::Retryable(format!(
            "all {} agents failed to start — cannot continue",
            startup.agents
        )));
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
/// borrowing `Config`. All respawn/refill paths use this. Its failure type
/// preserves whether a partially initialized child was proved shut down, so
/// the supervisor cannot turn cleanup ambiguity into a cooldown retry.
async fn spawn_and_init(
    command: &str,
    args: &[String],
    extra_env: &[(String, String)],
    has_generated_codex_config: bool,
    agent_index: usize,
    observer: Option<observer::ObserverHandle>,
    guard: &mut RespawnGuard,
) -> SpawnInitResult {
    let mut acp = AcpClient::spawn(command, args, extra_env, has_generated_codex_config)
        .await
        .map_err(|e| SpawnInitFailure::Spawn(anyhow::anyhow!("failed to spawn agent: {e}")))?;
    acp.set_observer(observer, agent_index);
    let initializing = Arc::new(tokio::sync::Mutex::new(Some(acp)));
    guard.mark_replacement_initializing(Arc::clone(&initializing));
    let mut slot = initializing.lock().await;

    let initialize_result = slot
        .as_mut()
        .expect("tracked replacement must own its client")
        .initialize()
        .await;
    match initialize_result {
        Ok(init_result) => {
            tracing::info!("agent initialized: {init_result}");
            let protocol_version = init_result["protocolVersion"].as_u64().unwrap_or(1) as u32;
            let acp = slot
                .take()
                .expect("initialized replacement must remain tracked");
            drop(slot);
            guard.mark_owner_cleanup_verified();
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
            // Drop is best-effort; replacement must know whether bounded
            // shutdown was actually verified.
            let cleanup = slot
                .as_mut()
                .expect("failed replacement must remain tracked")
                .shutdown()
                .await;
            let _ = slot.take();
            drop(slot);
            match cleanup {
                Ok(()) => {
                    guard.mark_owner_cleanup_verified();
                    Err(SpawnInitFailure::Spawn(anyhow::anyhow!(
                        "agent initialize failed: {e}"
                    )))
                }
                Err(cleanup_error) => {
                    guard.phase = RespawnPhase::OldOwnerUnverified;
                    Err(SpawnInitFailure::CleanupUnverified(anyhow::anyhow!(
                        "agent initialize failed: {e}; spawned adapter cleanup unverified: \
                         {cleanup_error}"
                    )))
                }
            }
        }
    }
}

async fn spawn_auth_client(agent: &AuthAgentArgs) -> Result<AcpClient, acp::AcpError> {
    let agent_args = config::normalize_agent_args(&agent.agent_command, agent.agent_args.clone());
    AcpClient::spawn(&agent.agent_command, &agent_args, &[], false).await
}

async fn shutdown_cli_client(client: &mut AcpClient, phase: &'static str) -> Result<()> {
    client
        .shutdown()
        .await
        .map_err(|error| anyhow::anyhow!("{phase}: adapter cleanup unverified: {error}"))
}

fn cli_failure_after_cleanup<T>(
    operation_error: anyhow::Error,
    cleanup_result: Result<()>,
) -> Result<T> {
    cleanup_result?;
    Err(operation_error)
}

async fn fail_cli_client<T>(
    client: &mut AcpClient,
    cleanup_phase: &'static str,
    operation_error: anyhow::Error,
) -> Result<T> {
    let cleanup_result = shutdown_cli_client(client, cleanup_phase).await;
    cli_failure_after_cleanup(operation_error, cleanup_result)
}

fn extract_auth_methods(init_result: &serde_json::Value) -> Vec<serde_json::Value> {
    init_result
        .get("authMethods")
        .and_then(|methods| methods.as_array())
        .cloned()
        .unwrap_or_default()
}

/// `buzz-acp auth-methods` — spawn an adapter, initialize it, print authMethods.
async fn run_auth_methods(args: AuthMethodsArgs, mut shutdown: watch::Receiver<()>) -> Result<()> {
    let mut client = spawn_auth_client(&args.agent)
        .await
        .map_err(|error| anyhow::anyhow!("failed to spawn agent: {error}"))?;

    let initialize = tokio::select! {
        biased;
        _ = shutdown.changed() => {
            shutdown_cli_client(&mut client, "auth-methods signal cleanup").await?;
            return Err(anyhow::anyhow!("auth-methods cancelled by shutdown signal"));
        }
        result = tokio::time::timeout(MODELS_TIMEOUT, client.initialize()) => result,
    };
    let init_result = match initialize {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            return fail_cli_client(
                &mut client,
                "auth-methods initialization failure",
                anyhow::anyhow!("agent initialize failed: {e}"),
            )
            .await;
        }
        Err(_) => {
            return fail_cli_client(
                &mut client,
                "auth-methods initialization timeout",
                anyhow::anyhow!("agent timed out ({MODELS_TIMEOUT:?})"),
            )
            .await;
        }
    };

    let methods = extract_auth_methods(&init_result);
    shutdown_cli_client(&mut client, "auth-methods completion").await?;

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
async fn run_authenticate(args: AuthenticateArgs, mut shutdown: watch::Receiver<()>) -> Result<()> {
    let mut client = spawn_auth_client(&args.agent)
        .await
        .map_err(|error| anyhow::anyhow!("failed to spawn agent: {error}"))?;

    let initialize = tokio::select! {
        biased;
        _ = shutdown.changed() => {
            shutdown_cli_client(&mut client, "authenticate signal cleanup").await?;
            return Err(anyhow::anyhow!("authenticate cancelled by shutdown signal"));
        }
        result = tokio::time::timeout(MODELS_TIMEOUT, client.initialize()) => result,
    };
    let init_result = match initialize {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            return fail_cli_client(
                &mut client,
                "authenticate initialization failure",
                anyhow::anyhow!("agent initialize failed: {e}"),
            )
            .await;
        }
        Err(_) => {
            return fail_cli_client(
                &mut client,
                "authenticate initialization timeout",
                anyhow::anyhow!("agent initialize timed out ({MODELS_TIMEOUT:?})"),
            )
            .await;
        }
    };

    let supports_method = extract_auth_methods(&init_result)
        .iter()
        .any(|method| method.get("id").and_then(|id| id.as_str()) == Some(args.method_id.as_str()));
    if !supports_method {
        return fail_cli_client(
            &mut client,
            "unsupported authenticate method",
            anyhow::anyhow!(
                "auth method '{}' is not advertised by this adapter",
                args.method_id
            ),
        )
        .await;
    }

    let result = tokio::select! {
        biased;
        _ = shutdown.changed() => {
            shutdown_cli_client(&mut client, "authenticate signal cleanup").await?;
            return Err(anyhow::anyhow!("authenticate cancelled by shutdown signal"));
        }
        result = tokio::time::timeout(
            AUTHENTICATE_TIMEOUT,
            client.authenticate(&args.method_id),
        ) => result,
    };

    match result {
        Ok(Ok(_)) => {
            shutdown_cli_client(&mut client, "authenticate completion").await?;
            Ok(())
        }
        Ok(Err(e)) => {
            fail_cli_client(
                &mut client,
                "authenticate failure",
                anyhow::anyhow!("authenticate failed: {e}"),
            )
            .await
        }
        Err(_) => {
            fail_cli_client(
                &mut client,
                "authenticate timeout",
                anyhow::anyhow!("authenticate timed out ({AUTHENTICATE_TIMEOUT:?})"),
            )
            .await
        }
    }
}

/// Flow: spawn → initialize → session/new → print models → shutdown.
/// No relay connection, no MCP servers, no subscriptions. ~2-5s total.
async fn run_models(args: ModelsArgs, mut shutdown: watch::Receiver<()>) -> Result<()> {
    use acp::{extract_model_config_options, extract_model_state};

    let agent_args = config::normalize_agent_args(&args.agent.agent_command, args.agent.agent_args);
    let cwd = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("/"))
        .to_string_lossy()
        .to_string();

    // Spawn outside the timeout so we always own the child for cleanup.
    // `models` subcommand doesn't use persona packs — no extra env, no codex config.
    let mut client = AcpClient::spawn(&args.agent.agent_command, &agent_args, &[], false)
        .await
        .map_err(|error| anyhow::anyhow!("failed to spawn agent: {error}"))?;

    // Initialize + session/new under a timeout. Client is owned above,
    // so shutdown() runs on all paths (success, error, timeout).
    let protocol_result = tokio::select! {
        biased;
        _ = shutdown.changed() => {
            shutdown_cli_client(&mut client, "models signal cleanup").await?;
            return Err(anyhow::anyhow!("models cancelled by shutdown signal"));
        }
        result = tokio::time::timeout(MODELS_TIMEOUT, async {
            let init = client.initialize().await?;
            let session = client.session_new_full(&cwd, vec![], None, None).await?;
            Ok::<_, acp::AcpError>((init, session))
        }) => result,
    };

    let (init_result, session_resp) = match protocol_result {
        Ok(Ok(tuple)) => tuple,
        Ok(Err(e)) => {
            return fail_cli_client(
                &mut client,
                "models protocol failure",
                anyhow::anyhow!("agent communication failed: {e}"),
            )
            .await;
        }
        Err(_) => {
            return fail_cli_client(
                &mut client,
                "models protocol timeout",
                anyhow::anyhow!("agent timed out ({MODELS_TIMEOUT:?})"),
            )
            .await;
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
    shutdown_cli_client(&mut client, "models completion").await?;

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
            &agent
        ));

        let wrong_kind = make_event(1, "!rotate", Some(&agent));
        assert!(!is_owner_control_command(&wrong_kind, 1, "!rotate", &agent));

        let wrong_content = make_event(KIND_STREAM_MESSAGE, "!cancel", Some(&agent));
        assert!(!is_owner_control_command(
            &wrong_content,
            KIND_STREAM_MESSAGE,
            "!rotate",
            &agent
        ));

        let no_mention = make_event(KIND_STREAM_MESSAGE, "!rotate", None);
        assert!(!is_owner_control_command(
            &no_mention,
            KIND_STREAM_MESSAGE,
            "!rotate",
            &agent
        ));
    }

    #[test]
    fn owner_control_execution_requires_relay_admission() {
        let agent = "ab".repeat(32);
        let event = make_event(KIND_STREAM_MESSAGE, "!shutdown", Some(&agent));

        assert!(
            !is_admitted_owner_control_command(
                &event,
                KIND_STREAM_MESSAGE,
                "!shutdown",
                &agent,
                false,
            ),
            "command shape alone must never authorize harness execution"
        );
        assert!(
            is_admitted_owner_control_command(
                &event,
                KIND_STREAM_MESSAGE,
                "!shutdown",
                &agent,
                true,
            ),
            "relay-admitted exact command shape remains executable"
        );
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
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: Some(control_tx),
                steer_tx: None,
            },
        );

        assert_eq!(
            signal_in_flight_task(&mut pool, other_channel_id, ControlSignal::Rotate),
            ControlSignalResult::NotAccepted
        );
        assert_eq!(
            signal_in_flight_task(&mut pool, channel_id, ControlSignal::Rotate),
            ControlSignalResult::Delivered
        );
        assert_eq!(
            pool.task_map()
                .values()
                .next()
                .and_then(|meta| meta.accepted_drop_control),
            Some(AcceptedDropControl::Rotate),
            "a successfully delivered rotate must persist its drop disposition"
        );
        assert_eq!(control_rx.await.unwrap(), ControlSignal::Rotate);
        assert_eq!(
            signal_in_flight_task(&mut pool, channel_id, ControlSignal::Rotate),
            ControlSignalResult::DropRecorded
        );
    }

    #[tokio::test]
    async fn signal_in_flight_task_records_cancel_drop_disposition() {
        let mut pool = AgentPool::from_slots(vec![]);
        let channel_id = Uuid::new_v4();
        let (control_tx, control_rx) = tokio::sync::oneshot::channel();

        let abort_handle = pool.join_set.spawn(async {});
        pool.task_map_mut().insert(
            abort_handle.id(),
            pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                turn_id: "cancel-drop".to_string(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: Some(control_tx),
                steer_tx: None,
            },
        );

        assert_eq!(
            signal_in_flight_task(&mut pool, channel_id, ControlSignal::Cancel),
            ControlSignalResult::Delivered
        );
        assert_eq!(
            pool.task_map()
                .values()
                .next()
                .and_then(|meta| meta.accepted_drop_control),
            Some(AcceptedDropControl::Cancel)
        );
        assert_eq!(control_rx.await.unwrap(), ControlSignal::Cancel);
    }

    #[tokio::test]
    async fn later_cancel_upgrades_batch_fate_after_interrupt_consumed_control_sender() {
        let mut pool = AgentPool::from_slots(vec![]);
        let channel_id = Uuid::new_v4();
        let (control_tx, control_rx) = tokio::sync::oneshot::channel();

        let abort_handle = pool.join_set.spawn(async {});
        pool.task_map_mut().insert(
            abort_handle.id(),
            pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                turn_id: "interrupt-then-cancel".to_string(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: Some(control_tx),
                steer_tx: None,
            },
        );

        assert_eq!(
            signal_in_flight_task(&mut pool, channel_id, ControlSignal::Interrupt),
            ControlSignalResult::Delivered
        );
        assert_eq!(control_rx.await.unwrap(), ControlSignal::Interrupt);
        assert_eq!(
            signal_in_flight_task(&mut pool, channel_id, ControlSignal::Cancel),
            ControlSignalResult::DropRecorded,
            "the owner drop disposition must still be recorded after the sender is consumed"
        );
        assert_eq!(
            pool.task_map()
                .values()
                .next()
                .and_then(|meta| meta.accepted_drop_control),
            Some(AcceptedDropControl::Cancel)
        );
    }

    #[tokio::test]
    async fn later_rotate_upgrades_batch_and_lifecycle_after_switch_consumed_control_sender() {
        let mut pool = AgentPool::from_slots(vec![]);
        let channel_id = Uuid::new_v4();
        let (control_tx, control_rx) = tokio::sync::oneshot::channel();

        let abort_handle = pool.join_set.spawn(async {});
        pool.task_map_mut().insert(
            abort_handle.id(),
            pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                turn_id: "switch-then-rotate".to_string(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: Some(control_tx),
                steer_tx: None,
            },
        );

        assert_eq!(
            signal_in_flight_task(
                &mut pool,
                channel_id,
                ControlSignal::SwitchModel(ModelSwitchRequest::new(
                    "runtime-model",
                    "0123456789abcdef0123456789abcdef",
                ))
            ),
            ControlSignalResult::Delivered
        );
        assert_eq!(
            control_rx.await.unwrap(),
            ControlSignal::SwitchModel(ModelSwitchRequest::new(
                "runtime-model",
                "0123456789abcdef0123456789abcdef",
            ))
        );
        assert_eq!(
            signal_in_flight_task(&mut pool, channel_id, ControlSignal::Rotate),
            ControlSignalResult::DropRecorded,
            "rotate must record upgraded batch fate and lifecycle after a prior control"
        );
        let meta = pool.task_map().values().next().unwrap();
        assert_eq!(
            meta.accepted_drop_control,
            Some(AcceptedDropControl::Rotate)
        );
        assert_eq!(
            meta.accepted_model_switch
                .as_ref()
                .map(|request| request.model_id.as_str()),
            Some("runtime-model"),
            "the earlier accepted model intent must survive the rotate upgrade"
        );
        assert_eq!(
            meta.accepted_model_switch
                .as_ref()
                .map(|request| request.request_id.as_str()),
            Some("0123456789abcdef0123456789abcdef"),
            "the exact request correlation must survive the rotate upgrade"
        );
    }

    #[tokio::test]
    async fn signal_in_flight_task_reports_closed_receiver_as_not_sent() {
        let mut pool = AgentPool::from_slots(vec![]);
        let channel_id = Uuid::new_v4();
        let (control_tx, control_rx) = tokio::sync::oneshot::channel();
        drop(control_rx);

        let abort_handle = pool.join_set.spawn(async {});
        pool.task_map_mut().insert(
            abort_handle.id(),
            pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                turn_id: "post-prompt-cleanup".to_string(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: Some(control_tx),
                steer_tx: None,
            },
        );

        assert_eq!(
            signal_in_flight_task(
                &mut pool,
                channel_id,
                ControlSignal::SwitchModel(ModelSwitchRequest::new(
                    "gpt-5",
                    "fedcba9876543210fedcba9876543210",
                ))
            ),
            ControlSignalResult::NotAccepted,
            "a dropped task receiver must not be reported as a delivered control"
        );
    }
}

#[cfg(test)]
mod model_switch_request_id_tests {
    use super::*;

    #[test]
    fn accepts_only_exact_lowercase_hex_request_ids() {
        assert_eq!(
            parse_model_switch_request_id(&serde_json::json!({
                "requestId": "0123456789abcdef0123456789abcdef"
            })),
            Some("0123456789abcdef0123456789abcdef")
        );

        for invalid in [
            serde_json::json!({}),
            serde_json::json!({"requestId": null}),
            serde_json::json!({"requestId": 7}),
            serde_json::json!({"requestId": "0123456789abcdef0123456789abcde"}),
            serde_json::json!({"requestId": "0123456789abcdef0123456789abcdef0"}),
            serde_json::json!({"requestId": "0123456789ABCDEF0123456789ABCDEF"}),
            serde_json::json!({"requestId": "0123456789abcdef0123456789abcdeg"}),
        ] {
            assert_eq!(
                parse_model_switch_request_id(&invalid),
                None,
                "invalid requestId must be rejected: {invalid}"
            );
        }
    }

    #[test]
    fn every_immediate_status_echoes_the_exact_request_id() {
        let request_id = "0123456789abcdef0123456789abcdef";
        for status in [
            "sent",
            "recycling",
            "switch_failed",
            "unsupported_model",
            "turn_ending",
            "no_active_turn",
        ] {
            let payload = switch_model_control_result_payload(status, "runtime-model", request_id);
            assert_eq!(payload["status"], status);
            assert_eq!(payload["modelId"], "runtime-model");
            assert_eq!(payload["requestId"], request_id);
        }
    }
}

#[cfg(test)]
mod observer_control_dedup_tests {
    use super::*;

    #[test]
    fn still_fresh_ids_are_retained_and_capacity_fails_closed() {
        let mut dedup = ObserverControlDedup::new(2);
        let now = 1_000;

        assert_eq!(
            dedup.admit("event-1".into(), now - 2, now),
            ObserverControlAdmission::Admitted
        );
        assert_eq!(
            dedup.admit("event-2".into(), now - 1, now),
            ObserverControlAdmission::Admitted
        );
        assert_eq!(
            dedup.admit("event-3".into(), now, now),
            ObserverControlAdmission::CapacityExceeded,
            "a full fresh window must reject new controls instead of forgetting replay IDs"
        );
        assert_eq!(
            dedup.admit("event-1".into(), now - 2, now),
            ObserverControlAdmission::Replay,
            "capacity pressure must not make a still-fresh replay admissible"
        );
        assert_eq!(dedup.order.len(), 2);
        assert_eq!(dedup.seen.len(), 2);
        assert_eq!(
            dedup.admit(
                "event-3".into(),
                now + OBSERVER_CONTROL_FRESHNESS_SECS + 1,
                now + OBSERVER_CONTROL_FRESHNESS_SECS + 1,
            ),
            ObserverControlAdmission::Admitted,
            "expired IDs should free capacity for fresh controls"
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

    #[tokio::test]
    async fn publisher_wrapper_releases_its_sender_and_drains_after_outer_drop() {
        let observer = observer::ObserverHandle::in_process();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let (publisher, _published_rx) = RelayEventPublisher::test_pair();
        observer.emit(
            "control_result",
            None,
            &observer::context_for(None, None, None),
            serde_json::json!({ "status": "shutdown" }),
        );

        let task = spawn_relay_observer_publisher(
            observer.clone(),
            publisher,
            agent_keys.clone(),
            agent_keys.public_key().to_hex(),
            owner_keys.public_key().to_hex(),
            owner_keys.public_key(),
        );
        drop(observer);

        assert!(
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .expect("publisher wrapper must observe closed lanes and drain")
                .expect("publisher wrapper must not panic"),
            "confirmed terminal delivery must preserve successful shutdown verification"
        );
    }

    /// An event emitted between `subscribe()` and `snapshot()` lands in BOTH
    /// the snapshot and the live receiver; exact snapshot membership must
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

    #[tokio::test(start_paused = true)]
    async fn snapshot_membership_does_not_drop_an_earlier_uncaptured_control_result() {
        let observer = observer::ObserverHandle::in_process();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let (publisher, mut published_rx) = RelayEventPublisher::test_pair();
        let rx = observer.subscribe();

        observer.emit(
            "control_result",
            None,
            &observer::context_for(None, None, None),
            serde_json::json!({ "status": "shutdown" }),
        );
        emit_marker(&observer, "captured-telemetry");
        let snapshot: Vec<_> = observer
            .snapshot()
            .into_iter()
            .filter(|event| event.kind == "test_event")
            .collect();
        assert_eq!(snapshot.len(), 1, "the synthetic snapshot contains only T");
        drop(observer);

        let terminal_delivery_ok = run_relay_observer_publisher(
            snapshot,
            rx,
            publisher,
            agent_keys.clone(),
            agent_keys.public_key().to_hex(),
            owner_keys.public_key().to_hex(),
            owner_keys.public_key(),
        )
        .await;

        let mut published_kinds = Vec::new();
        while let Some(event) = published_rx.recv().await {
            let payload: serde_json::Value =
                decrypt_observer_payload(&owner_keys, &event).expect("decrypt published frame");
            published_kinds.push(payload["kind"].as_str().unwrap().to_string());
        }
        assert_eq!(
            published_kinds
                .iter()
                .filter(|kind| kind.as_str() == "control_result")
                .count(),
            1,
            "C must publish exactly once even though the later T was the only snapshot member"
        );
        assert!(
            terminal_delivery_ok,
            "confirmed terminal delivery must preserve successful shutdown verification"
        );
    }

    #[tokio::test]
    async fn control_result_broadcast_lag_fails_shutdown_delivery_verification() {
        let observer = observer::ObserverHandle::in_process();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let (publisher, _published_rx) = RelayEventPublisher::test_pair();
        let rx = observer.subscribe();

        // The reserved lane holds 64 results. A subscribed receiver that does
        // not poll until the 65th result must report Lagged(1), and that
        // uncertainty must survive the remaining drain as a false verdict.
        for marker in 0..65 {
            observer.emit(
                "control_result",
                None,
                &observer::context_for(None, None, None),
                serde_json::json!({ "marker": marker }),
            );
        }
        drop(observer);

        let terminal_delivery_ok = run_relay_observer_publisher(
            Vec::new(),
            rx,
            publisher,
            agent_keys.clone(),
            agent_keys.public_key().to_hex(),
            owner_keys.public_key().to_hex(),
            owner_keys.public_key(),
        )
        .await;

        assert!(
            !terminal_delivery_ok,
            "priority-lane loss must fail closed even when the remaining results drain"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn live_control_result_preempts_captured_telemetry_pacing() {
        let observer = observer::ObserverHandle::in_process();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let (publisher, mut published_rx) = RelayEventPublisher::test_pair();

        for marker in 0..48 {
            emit_marker(&observer, &marker.to_string());
        }
        let rx = observer.subscribe();
        let snapshot = observer.snapshot();
        observer.emit(
            "control_result",
            None,
            &observer::context_for(None, None, None),
            serde_json::json!({ "status": "shutdown" }),
        );

        let task = tokio::spawn(run_relay_observer_publisher(
            snapshot,
            rx,
            publisher,
            agent_keys.clone(),
            agent_keys.public_key().to_hex(),
            owner_keys.public_key().to_hex(),
            owner_keys.public_key(),
        ));

        tokio::time::advance(Duration::from_secs(1)).await;
        let event = published_rx
            .recv()
            .await
            .expect("priority control result must publish before the snapshot drains");
        let payload: serde_json::Value =
            decrypt_observer_payload(&owner_keys, &event).expect("decrypt published frame");
        assert_eq!(payload["kind"], "control_result");
        assert_eq!(payload["payload"]["status"], "shutdown");

        task.abort();
        let _ = task.await;
    }

    #[tokio::test(start_paused = true)]
    async fn live_control_result_preempts_live_chunk_flush_pacing() {
        let observer = observer::ObserverHandle::in_process();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let (publisher, mut published_rx) = RelayEventPublisher::test_pair();
        let rx = observer.subscribe();
        let snapshot = observer.snapshot();
        observer.emit(
            "acp_read",
            None,
            &observer::context_for(None, None, None),
            serde_json::json!({
                "params": {
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "messageId": "message-1",
                        "content": {"type": "text", "text": "chunk"},
                    },
                },
            }),
        );

        let task = tokio::spawn(run_relay_observer_publisher(
            snapshot,
            rx,
            publisher,
            agent_keys.clone(),
            agent_keys.public_key().to_hex(),
            owner_keys.public_key().to_hex(),
            owner_keys.public_key(),
        ));
        tokio::time::advance(Duration::from_millis(100)).await;
        assert!(
            published_rx.try_recv().is_err(),
            "the live chunk flush must still be in ordinary pacing"
        );

        observer.emit(
            "control_result",
            None,
            &observer::context_for(None, None, None),
            serde_json::json!({ "status": "shutdown" }),
        );
        tokio::time::advance(Duration::from_millis(100)).await;
        let event = published_rx
            .recv()
            .await
            .expect("priority control result must interrupt live chunk pacing");
        let payload: serde_json::Value =
            decrypt_observer_payload(&owner_keys, &event).expect("decrypt published frame");
        assert_eq!(payload["kind"], "control_result");
        assert_eq!(payload["payload"]["status"], "shutdown");

        task.abort();
        let _ = task.await;
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
            context_message_limit: 12,
            max_turns_per_session: 0,
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
            lazy_pool: false,
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
    use crate::acp::{AcpClient, AcpError, StopReason};
    use crate::observer::ObserverHandle;
    use crate::pool::{
        AgentPool, ChannelInfoResolver, ControlSignal, OwnedAgent, PromptContext, PromptOutcome,
        PromptResult, PromptSource, SessionState, TimeoutKind,
    };
    use crate::queue::{BatchEvent, BatchOccurrenceIds, FlushBatch};
    use crate::relay::{ChannelInfo, RestClient};
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
            context_message_limit: 12,
            max_turns_per_session: 0,
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
            lazy_pool: false,
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
            model_switch_request_id: None,
            model_switch_rollback: None,
            agent_name: "unknown".into(),
            goose_system_prompt_supported: None,
            // Error branches under test never read this; 1 is the legacy
            // non-systemPrompt path, the simplest valid value.
            protocol_version: 1,
        }
    }

    #[tokio::test]
    async fn shutdown_cooperatively_recovers_and_reaps_active_prompt_and_heartbeat_agents() {
        let mut pool = AgentPool::from_slots(vec![None, None]);
        for (index, source) in [
            (0, PromptSource::Channel(Uuid::new_v4())),
            (1, PromptSource::Heartbeat),
        ] {
            let agent = dummy_agent(index).await;
            let result_tx = pool.result_tx();
            let (control_tx, control_rx) = tokio::sync::oneshot::channel();
            let channel_id = match &source {
                PromptSource::Channel(channel_id) => Some(*channel_id),
                PromptSource::Heartbeat => None,
            };
            let task_source = source;
            let abort_handle = pool.join_set.spawn(async move {
                let signal = control_rx.await.expect("shutdown control");
                assert!(matches!(signal, ControlSignal::Cancel));
                let _ = result_tx.send(PromptResult {
                    agent,
                    source: task_source,
                    turn_id: format!("shutdown-turn-{index}"),
                    outcome: PromptOutcome::Cancelled,
                    batch: None,
                    retry_agent_index: None,
                });
            });
            pool.task_map_mut().insert(
                abort_handle.id(),
                crate::pool::TaskMeta {
                    agent_index: index,
                    channel_id,
                    turn_id: format!("shutdown-turn-{index}"),
                    recoverable_batch: None,
                    desired_model: None,
                    model_overridden: false,
                    accepted_model_switch: None,
                    accepted_drop_control: None,
                    control_tx: Some(control_tx),
                    steer_tx: None,
                },
            );
        }

        assert!(
            tokio::time::timeout(Duration::from_secs(5), shutdown_agent_pool(&mut pool))
                .await
                .expect("cooperative shutdown must be bounded"),
            "every typed agent owner must be explicitly reaped with verified process-group absence"
        );
        assert!(pool.join_set.is_empty());
    }

    fn register_completing_task(pool: &mut AgentPool, agent_index: usize, channel_id: Uuid) {
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index,
                channel_id: Some(channel_id),
                turn_id: format!("turn-{agent_index}"),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );
    }

    fn one_event_batch(channel_id: Uuid, content: &str) -> FlushBatch {
        FlushBatch {
            channel_id,
            events: vec![BatchEvent {
                event: EventBuilder::new(Kind::Custom(9), content)
                    .sign_with_keys(&Keys::generate())
                    .unwrap(),
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: Some(CancelReason::Interrupt),
            occurrence_ids: BatchOccurrenceIds::for_test(1, 0),
        }
    }

    #[test]
    fn ordinary_pool_capacity_deferral_preserves_retry_budget() {
        let channel_id = Uuid::new_v4();
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        queue.push(crate::queue::QueuedEvent {
            channel_id,
            event: EventBuilder::new(Kind::Custom(9), "capacity deferred")
                .sign_with_keys(&Keys::generate())
                .unwrap(),
            received_at: std::time::Instant::now(),
            prompt_tag: "test".into(),
        });

        let deferred = queue.flush_next().expect("queued batch");
        queue.set_retry_count_for_test(channel_id, crate::queue::MAX_RETRIES);
        defer_unpinned_batch_for_capacity(&mut queue, deferred);

        let final_attempt = queue.flush_next().expect("deferred batch");
        assert!(
            queue.requeue(final_attempt).is_some(),
            "capacity deferral must not replenish an exhausted poison-batch retry budget"
        );
    }

    #[tokio::test]
    async fn control_preempted_setup_requeues_as_cancelled_without_charging_retry_budget() {
        let channel_id = Uuid::new_v4();
        let original = EventBuilder::new(Kind::Custom(9), "setup-preempted")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![None]);
        register_completing_task(&mut pool, 0, channel_id);
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        queue.push(QueuedEvent {
            channel_id,
            event: original.clone(),
            received_at: std::time::Instant::now(),
            prompt_tag: "test".into(),
        });
        let mut batch = queue.flush_next().expect("active setup batch");
        batch.cancel_reason = Some(CancelReason::Interrupt);
        queue.set_retry_count_for_test(channel_id, crate::queue::MAX_RETRIES);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(2);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        let action = handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "turn-setup-preempted".into(),
                outcome: PromptOutcome::ControlPreemptedSetup,
                batch: Some(batch),
                retry_agent_index: None,
            },
            &mut heartbeat_in_flight,
            &removed_channels,
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
        );

        assert!(matches!(action, LoopAction::Continue));
        let requeued = queue
            .flush_next()
            .expect("control-preempted setup must preserve the batch");
        let preserved_ids = requeued
            .events
            .iter()
            .chain(requeued.cancelled_events.iter())
            .map(|event| event.event.id)
            .collect::<Vec<_>>();
        assert_eq!(
            preserved_ids,
            [original.id],
            "even an exhausted retry budget must preserve the setup-preempted event exactly once"
        );
        assert_eq!(requeued.cancel_reason, Some(CancelReason::Interrupt));
        assert_eq!(
            crash_history[0].crash_times.len(),
            0,
            "cooperative setup preemption must not consume crash budget"
        );

        respawn_tasks.shutdown().await;
    }

    #[tokio::test]
    async fn busy_switch_model_retry_pins_original_slot_not_other_idle_agent() {
        let channel_id = Uuid::new_v4();
        let mut agent = dummy_agent(0).await;
        agent.desired_model = Some("runtime-model".into());
        agent.model_overridden = true;
        let other = dummy_agent(1).await;
        let mut pool = AgentPool::from_slots(vec![None, Some(other)]);
        register_completing_task(&mut pool, 0, channel_id);
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let mut crash_history = vec![SlotCircuit::new(), SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(2);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        let action = handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "turn-0".into(),
                outcome: PromptOutcome::Cancelled,
                batch: Some(one_event_batch(channel_id, "switch-model-retry")),
                retry_agent_index: Some(0),
            },
            &mut heartbeat_in_flight,
            &HashSet::new(),
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
        );

        assert!(action == LoopAction::Continue);
        assert_eq!(queue.required_agent(channel_id), Some(0));
        assert!(
            pool.slot_alive(1),
            "unrelated idle slot must stay unclaimed"
        );
        let exact = pool
            .try_claim_index(0)
            .expect("the replay must claim its original slot");
        assert_eq!(exact.desired_model.as_deref(), Some("runtime-model"));
        pool.return_agent(exact);
        shutdown_agent_pool(&mut pool).await;
    }

    #[tokio::test]
    async fn post_model_setup_exit_preserves_pending_switch_for_replacement_retry() {
        let channel_id = Uuid::new_v4();
        let mut agent = dummy_agent(0).await;
        agent.desired_model = Some("model-b".into());
        agent.model_overridden = true;
        agent.model_switch_request_id = Some("0123456789abcdef0123456789abcdef".into());
        agent.model_switch_rollback = Some(Box::new(ModelSwitchRollback {
            desired_model: Some("model-a".into()),
            model_overridden: false,
            request_id: None,
            previous: None,
        }));
        let mut pool = AgentPool::from_slots(vec![None]);
        register_completing_task(&mut pool, 0, channel_id);
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        let action = handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "turn-post-model-setup-exit".into(),
                outcome: PromptOutcome::AgentExited,
                batch: Some(one_event_batch(channel_id, "retry pending switch")),
                retry_agent_index: None,
            },
            &mut heartbeat_in_flight,
            &HashSet::new(),
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
        );

        assert!(action == LoopAction::Continue);
        assert_eq!(
            queue.required_agent(channel_id),
            Some(0),
            "the requeued turn must stay pinned to the slot carrying its pending switch"
        );
        let pending = crash_history[0]
            .pending_model_intent
            .as_ref()
            .expect("fatal setup failure must retain pending switch state");
        assert_eq!(pending.desired_model.as_deref(), Some("model-b"));
        assert!(pending.model_overridden);
        assert_eq!(
            pending.model_switch_request_id.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        let rollback = pending
            .model_switch_rollback
            .as_ref()
            .expect("replacement retry must retain prior-model rollback");
        assert_eq!(rollback.desired_model.as_deref(), Some("model-a"));
        assert!(!rollback.model_overridden);

        respawn_tasks.shutdown().await;
    }

    #[tokio::test]
    async fn successful_session_retirement_keeps_override_pin_until_next_turn_succeeds() {
        let channel_id = Uuid::new_v4();
        let mut agent = dummy_agent(0).await;
        agent.desired_model = Some("runtime-model".into());
        agent.model_overridden = true;
        let mut pool = AgentPool::from_slots(vec![None]);
        register_completing_task(&mut pool, 0, channel_id);
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "turn-0".into(),
                outcome: PromptOutcome::SessionRetired(StopReason::EndTurn),
                batch: None,
                retry_agent_index: None,
            },
            &mut heartbeat_in_flight,
            &HashSet::new(),
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
        );
        assert_eq!(queue.required_agent(channel_id), Some(0));

        let replay_agent = pool.try_claim_index(0).expect("pinned replay agent");
        register_completing_task(&mut pool, 0, channel_id);
        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent: replay_agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "turn-0-replay".into(),
                outcome: PromptOutcome::Ok(StopReason::EndTurn),
                batch: None,
                retry_agent_index: None,
            },
            &mut heartbeat_in_flight,
            &HashSet::new(),
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
        );
        assert_eq!(
            queue.required_agent(channel_id),
            None,
            "a successful fresh-session turn can fall back to session affinity"
        );
        shutdown_agent_pool(&mut pool).await;
    }

    #[tokio::test]
    async fn completed_before_switch_model_keeps_no_batch_pin_and_healthy_return_wakes_it() {
        let source_channel = Uuid::new_v4();
        let waiting_channel = Uuid::new_v4();
        let mut agent = dummy_agent(0).await;
        agent.desired_model = Some("runtime-model".into());
        agent.model_overridden = true;
        let mut pool = AgentPool::from_slots(vec![None]);
        register_completing_task(&mut pool, 0, source_channel);
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        queue.push(QueuedEvent {
            channel_id: waiting_channel,
            event: EventBuilder::new(Kind::Custom(9), "waiting")
                .sign_with_keys(&Keys::generate())
                .unwrap(),
            received_at: std::time::Instant::now(),
            prompt_tag: "test".into(),
        });
        queue.require_agent(waiting_channel, 0);
        queue.block_required_agent(waiting_channel);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent,
                source: PromptSource::Channel(source_channel),
                turn_id: "turn-0".into(),
                outcome: PromptOutcome::Ok(StopReason::EndTurn),
                batch: None,
                retry_agent_index: Some(0),
            },
            &mut heartbeat_in_flight,
            &HashSet::new(),
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
        );

        assert_eq!(queue.required_agent(source_channel), Some(0));
        assert_eq!(
            queue
                .flush_next()
                .expect("healthy slot return must wake other pinned work")
                .channel_id,
            waiting_channel
        );
        shutdown_agent_pool(&mut pool).await;
    }

    #[tokio::test]
    async fn acknowledged_switch_model_is_reconciled_when_prompt_result_wins_ready_race() {
        let channel_id = Uuid::new_v4();
        let mut agent = dummy_agent(0).await;
        agent
            .state
            .sessions
            .insert(channel_id, "session-before-switch".into());
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                turn_id: "turn-ready-race".into(),
                recoverable_batch: None,
                desired_model: Some("runtime-model".into()),
                model_overridden: true,
                accepted_model_switch: Some(ModelSwitchRequest::new(
                    "runtime-model",
                    "0123456789abcdef0123456789abcdef",
                )),
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "turn-ready-race".into(),
                outcome: PromptOutcome::Ok(StopReason::EndTurn),
                batch: None,
                retry_agent_index: None,
            },
            &mut heartbeat_in_flight,
            &HashSet::new(),
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
        );

        assert_eq!(
            queue.required_agent(channel_id),
            Some(0),
            "an acknowledged switch must remain pinned to the accepting slot"
        );
        assert_eq!(
            pool.live_count(),
            0,
            "the adapter still owns the old session and must not return idle"
        );
        assert!(
            crash_history[0].crash_times.is_empty(),
            "an acknowledged lifecycle recycle must not consume crash budget"
        );
        assert_eq!(respawn_tasks.len(), 1);
        assert_eq!(
            crash_history[0]
                .pending_model_intent
                .as_ref()
                .and_then(|intent| intent.desired_model.as_deref()),
            Some("runtime-model")
        );
        assert_eq!(
            crash_history[0]
                .pending_model_intent
                .as_ref()
                .and_then(|intent| intent.model_switch_request_id.as_deref()),
            Some("0123456789abcdef0123456789abcdef")
        );
        respawn_tasks.shutdown().await;
    }

    #[tokio::test]
    async fn unsupported_switch_model_ready_race_restores_prior_intent_without_recycle() {
        let channel_id = Uuid::new_v4();
        let mut agent = dummy_agent(0).await;
        let observer = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(observer.clone()), 0);
        agent
            .state
            .sessions
            .insert(channel_id, "session-before-switch".into());
        agent.model_capabilities = Some(pool::AgentModelCapabilities {
            config_options_raw: vec![serde_json::json!({
                "configId": "model",
                "category": "model",
                "options": [{"value": "model-a"}],
            })],
            available_models_raw: None,
        });
        agent.desired_model = Some("model-a".into());
        agent.model_overridden = true;

        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                turn_id: "turn-unsupported-ready-race".into(),
                recoverable_batch: None,
                // The sender currently records the accepted request here for
                // panic recovery. The returned agent remains the authority for
                // the prior, already-applied model intent.
                desired_model: Some("model-b".into()),
                model_overridden: true,
                accepted_model_switch: Some(ModelSwitchRequest::new(
                    "model-b",
                    "abcdef0123456789abcdef0123456789",
                )),
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "turn-unsupported-ready-race".into(),
                outcome: PromptOutcome::Ok(StopReason::EndTurn),
                batch: None,
                retry_agent_index: None,
            },
            &mut heartbeat_in_flight,
            &HashSet::new(),
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
        );

        let live_count = pool.live_count();
        let required_agent = queue.required_agent(channel_id);
        let respawn_count = respawn_tasks.len();
        let mut returned_agent = pool.try_claim_index(0);
        let returned_intent = returned_agent.as_ref().map(|agent| {
            (
                agent.desired_model.clone(),
                agent.model_overridden,
                agent.state.sessions.get(&channel_id).cloned(),
            )
        });
        if let Some(agent) = returned_agent.as_mut() {
            agent
                .acp
                .shutdown()
                .await
                .expect("returned adapter must shut down cleanly");
        }
        respawn_tasks.shutdown().await;

        assert_eq!(
            returned_intent,
            Some((
                Some("model-a".to_string()),
                true,
                Some("session-before-switch".to_string()),
            )),
            "unsupported model B must preserve the prior applied model A and session"
        );
        assert_eq!(live_count, 1, "the healthy prior-model adapter stays idle");
        assert_eq!(
            required_agent, None,
            "unsupported model B must not create an exact-slot retry"
        );
        assert_eq!(
            respawn_count, 0,
            "unsupported model B must not recycle the healthy adapter"
        );
        let unsupported = observer
            .snapshot()
            .into_iter()
            .find(|event| event.kind == "control_result")
            .expect("ready-race rejection must emit a terminal control result");
        assert_eq!(unsupported.payload["status"], "unsupported_model");
        assert_eq!(unsupported.payload["modelId"], "model-b");
        assert_eq!(
            unsupported.payload["requestId"],
            "abcdef0123456789abcdef0123456789"
        );
    }

    #[tokio::test]
    async fn acknowledged_rotate_recycles_when_prompt_result_wins_ready_race() {
        let channel_id = Uuid::new_v4();
        let mut agent = dummy_agent(0).await;
        agent
            .state
            .sessions
            .insert(channel_id, "session-before-rotate".into());
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                turn_id: "turn-rotate-ready-race".into(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: Some(AcceptedDropControl::Rotate),
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "turn-rotate-ready-race".into(),
                outcome: PromptOutcome::Ok(StopReason::EndTurn),
                batch: None,
                retry_agent_index: None,
            },
            &mut heartbeat_in_flight,
            &HashSet::new(),
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
        );

        assert_eq!(pool.live_count(), 0);
        assert_eq!(respawn_tasks.len(), 1);
        assert!(crash_history[0].crash_times.is_empty());
        respawn_tasks.shutdown().await;
    }

    #[tokio::test]
    async fn acknowledged_rotate_drops_batch_when_error_wins_ready_race() {
        let channel_id = Uuid::new_v4();
        let mut agent = dummy_agent(0).await;
        agent
            .state
            .sessions
            .insert(channel_id, "session-before-rotate".into());
        let mut batch = one_event_batch(channel_id, "must-not-replay");
        batch.cancel_reason = None;
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                turn_id: "turn-rotate-error-race".into(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: Some(AcceptedDropControl::Rotate),
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "turn-rotate-error-race".into(),
                outcome: PromptOutcome::Error(AcpError::AgentError {
                    code: -32000,
                    message: "application failure won ready race".into(),
                }),
                batch: Some(batch),
                retry_agent_index: None,
            },
            &mut heartbeat_in_flight,
            &HashSet::new(),
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
        );

        assert_eq!(
            queue.pending_channels(),
            0,
            "an accepted rotate must drop the losing prompt outcome's batch"
        );
        assert_eq!(pool.live_count(), 0);
        assert_eq!(respawn_tasks.len(), 1);
        assert!(crash_history[0].crash_times.is_empty());
        respawn_tasks.shutdown().await;
    }

    #[tokio::test]
    async fn acknowledged_cancel_drops_batch_when_error_wins_ready_race() {
        let channel_id = Uuid::new_v4();
        let agent = dummy_agent(0).await;
        let mut batch = one_event_batch(channel_id, "must-not-replay");
        batch.cancel_reason = None;
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                turn_id: "turn-cancel-ready-race".into(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: Some(AcceptedDropControl::Cancel),
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "turn-cancel-ready-race".into(),
                outcome: PromptOutcome::Error(AcpError::AgentError {
                    code: -32000,
                    message: "application failure won ready race".into(),
                }),
                batch: Some(batch),
                retry_agent_index: None,
            },
            &mut heartbeat_in_flight,
            &HashSet::new(),
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
        );

        assert_eq!(
            queue.pending_channels(),
            0,
            "an accepted cancel must dominate the losing prompt outcome's retry policy"
        );
        assert_eq!(pool.live_count(), 1);
        shutdown_agent_pool(&mut pool).await;
    }

    #[tokio::test]
    async fn membership_removed_while_checked_out_recycles_remote_session_owner() {
        let channel_id = Uuid::new_v4();
        let mut agent = dummy_agent(0).await;
        agent
            .state
            .sessions
            .insert(channel_id, "session-before-removal".into());
        let mut pool = AgentPool::from_slots(vec![None]);
        register_completing_task(&mut pool, 0, channel_id);
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        handle_prompt_result(
            &mut pool,
            &mut queue,
            &config,
            PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "turn-membership-removed".into(),
                outcome: PromptOutcome::Ok(StopReason::EndTurn),
                batch: None,
                retry_agent_index: None,
            },
            &mut heartbeat_in_flight,
            &HashSet::from([channel_id]),
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            None,
            None,
        );

        assert_eq!(pool.live_count(), 0);
        assert_eq!(respawn_tasks.len(), 1);
        assert!(crash_history[0].crash_times.is_empty());
        assert_eq!(queue.required_agent(channel_id), None);
        respawn_tasks.shutdown().await;
    }

    #[tokio::test]
    async fn rejected_session_close_preserves_batch_and_respawns_poisoned_adapter() {
        let capture =
            std::env::temp_dir().join(format!("buzz-acp-close-rejected-{}.ndjson", Uuid::new_v4()));
        let script = r#"
            capture="$1"
            IFS= read -r initialize
            printf '%s\n' "$initialize" >> "$capture"
            printf '%s\n' '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":2,"agentCapabilities":{"sessionCapabilities":{"close":{}}}}}'
            IFS= read -r prompt
            printf '%s\n' "$prompt" >> "$capture"
            IFS= read -r cancel
            printf '%s\n' "$cancel" >> "$capture"
            printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"stopReason":"cancelled"}}'
            IFS= read -r close
            printf '%s\n' "$close" >> "$capture"
            printf '%s\n' '{"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"Method not found"}}'
            sleep 10
        "#;
        let mut acp = AcpClient::spawn(
            "bash",
            &[
                "-c".to_string(),
                script.to_string(),
                "buzz-acp-close-rejected-test".to_string(),
                capture.to_string_lossy().into_owned(),
            ],
            &[],
            false,
        )
        .await
        .expect("failed to spawn fake ACP adapter");
        acp.initialize()
            .await
            .expect("fake adapter must advertise session/close");
        assert!(acp.supports_session_close());

        let channel_id = Uuid::new_v4();
        let mut state = SessionState::default();
        state.sessions.insert(channel_id, "session-old".into());
        let agent = OwnedAgent {
            index: 0,
            acp,
            state,
            model_capabilities: None,
            desired_model: None,
            model_overridden: false,
            model_switch_request_id: None,
            model_switch_rollback: None,
            agent_name: "claude-code-acp".into(),
            goose_system_prompt_supported: None,
            protocol_version: 2,
        };
        let keys = Keys::generate();
        let rest_client = RestClient {
            http: reqwest::Client::new(),
            base_url: "http://127.0.0.1:0".into(),
            keys: keys.clone(),
            auth_tag_json: None,
        };
        let ctx = Arc::new(PromptContext {
            mcp_servers: vec![],
            initial_message: None,
            idle_timeout: Duration::from_secs(60),
            max_turn_duration: Duration::from_secs(120),
            turn_liveness_interval: Duration::ZERO,
            dedup_mode: config::DedupMode::Queue,
            system_prompt: None,
            session_title: None,
            team_instructions: None,
            heartbeat_prompt: None,
            base_prompt: None,
            cwd: ".".into(),
            rest_client: rest_client.clone(),
            channel_info: ChannelInfoResolver::new(
                std::collections::HashMap::from([(
                    channel_id,
                    ChannelInfo {
                        name: "test".into(),
                        channel_type: "public".into(),
                    },
                )]),
                rest_client,
            ),
            context_message_limit: 0,
            max_turns_per_session: 0,
            permission_mode: config::PermissionMode::Default,
            agent_keys: keys.clone(),
            agent_owner_pubkey: None,
            memory_enabled: false,
            harness_name: "claude-code-acp".into(),
            relay_url: "ws://127.0.0.1:3000".into(),
        });
        let event = EventBuilder::new(Kind::Custom(9), "original")
            .sign_with_keys(&keys)
            .unwrap();
        let event_id = event.id;
        let batch = FlushBatch {
            channel_id,
            events: vec![BatchEvent {
                event,
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
            occurrence_ids: BatchOccurrenceIds::for_test(1, 0),
        };
        let (result_tx, mut result_rx) = mpsc::unbounded_channel();
        let (control_tx, control_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(crate::pool::run_prompt_task(
            agent,
            Some(batch),
            Some("prompt".into()),
            ctx,
            result_tx,
            Some(control_rx),
            "turn-close-rejected".into(),
        ));

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if std::fs::read_to_string(&capture)
                    .is_ok_and(|contents| contents.lines().count() >= 2)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("prompt request must reach the fake adapter");
        control_tx
            .send(ControlSignal::Steer)
            .expect("control receiver must still be live");
        let result = tokio::time::timeout(Duration::from_secs(5), result_rx.recv())
            .await
            .expect("prompt task must return after close rejection")
            .expect("prompt result channel must stay open");
        task.await.expect("prompt task must not panic");
        assert!(matches!(
            result.outcome,
            PromptOutcome::SessionCloseFailed(AcpError::AgentError { code: -32601, .. })
        ));
        let captured = std::fs::read_to_string(&capture).expect("capture must remain readable");
        let messages: Vec<serde_json::Value> = captured
            .lines()
            .map(|line| serde_json::from_str(line).expect("captured line must be JSON"))
            .collect();
        assert_eq!(messages[0]["method"], "initialize");
        assert_eq!(messages[1]["method"], "session/prompt");
        assert_eq!(messages[2]["method"], "session/cancel");
        assert_eq!(messages[3]["method"], "session/close");

        assert_eq!(
            result
                .agent
                .state
                .sessions
                .get(&channel_id)
                .map(String::as_str),
            Some("session-old"),
            "failed close must not silently discard local session ownership"
        );

        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                turn_id: "turn-close-rejected".into(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();

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
        );

        assert_eq!(
            pool.live_count(),
            0,
            "an adapter with uncertain remote ownership must never return idle"
        );
        assert_eq!(
            respawn_tasks.len(),
            1,
            "close rejection must enter process-group shutdown and respawn"
        );
        let requeued = queue
            .flush_next()
            .expect("steer-cancelled work must remain queued");
        assert_eq!(
            requeued.events.len(),
            1,
            "with no newer event to merge, the queue re-dispatches the cancelled batch"
        );
        assert!(requeued.cancelled_events.is_empty());
        assert_eq!(requeued.events[0].event.id, event_id);
        assert_eq!(requeued.cancel_reason, Some(CancelReason::Steer));

        let _ = std::fs::remove_file(capture);
    }

    struct OutcomeDisposition {
        turn_errors: usize,
        live_agents: usize,
        respawn_tasks: usize,
        recent_crashes: usize,
        circuit_open: bool,
        turn_error_texts: Vec<String>,
    }

    /// Drive one outcome through `handle_prompt_result` and report its
    /// observable supervisor disposition.
    async fn outcome_disposition_for(outcome: PromptOutcome) -> OutcomeDisposition {
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
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );

        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let observer = ObserverHandle::in_process();

        let result = PromptResult {
            agent,
            source: PromptSource::Channel(Uuid::new_v4()),
            turn_id: "test-turn-id".to_string(),
            outcome,
            batch: None,
            retry_agent_index: None,
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
        OutcomeDisposition {
            turn_errors: turn_errors.len(),
            live_agents: pool.live_count(),
            respawn_tasks: respawn_tasks.len(),
            recent_crashes: crash_history[0].crash_times.len(),
            circuit_open: crash_history[0].open_until.is_some(),
            turn_error_texts: turn_errors
                .iter()
                .filter_map(|event| event.payload["error"].as_str().map(str::to_owned))
                .collect(),
        }
    }

    /// Drive one error outcome through `handle_prompt_result` and return how
    /// many `turn_error` events it emitted to the observer feed.
    async fn turn_errors_emitted_for(outcome: PromptOutcome) -> usize {
        outcome_disposition_for(outcome).await.turn_errors
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
                turn_id: "panic-turn-id".to_string(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
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
        let mut crash_history = vec![SlotCircuit::new()];
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
            None,
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

    #[test]
    fn cleanup_unverified_quarantine_cannot_be_bypassed_by_refill_cooldown() {
        let mut slot = SlotCircuit::new();
        slot.mark_cleanup_unverified();
        slot.open_until = Some(std::time::Instant::now() - Duration::from_secs(1));

        assert!(
            !slot.can_refill(),
            "elapsed crash cooldown must not authorize overlap after cleanup became unverified"
        );
        assert!(
            slot.blocks_supervisor_exit(),
            "the process must remain alive and degraded instead of restarting into possible overlap"
        );
    }

    #[tokio::test]
    async fn automatic_exit_waits_in_quarantine_until_explicit_shutdown() {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        let mut wait = Box::pin(await_automatic_exit_permission(
            &mut shutdown_rx,
            true,
            "deterministic test exit",
        ));

        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut wait)
                .await
                .is_err(),
            "automatic failure must not let the supervisor exit while cleanup is unverified"
        );

        shutdown_tx.send(()).expect("explicit shutdown");
        tokio::time::timeout(Duration::from_secs(1), wait)
            .await
            .expect("explicit shutdown releases quarantine");
    }

    #[tokio::test]
    async fn accepted_cancel_drops_recoverable_batch_when_task_panics() {
        let mut pool = AgentPool::from_slots(vec![None]);
        let channel_id = Uuid::new_v4();
        let mut batch = one_event_batch(channel_id, "must-not-replay");
        batch.cancel_reason = None;
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
                turn_id: "panic-after-cancel".into(),
                recoverable_batch: Some(batch),
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: Some(AcceptedDropControl::Cancel),
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
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

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
            None,
            None,
        );

        assert_eq!(
            queue.pending_channels(),
            0,
            "panic recovery must not resurrect work after an accepted cancel"
        );
        respawn_tasks.shutdown().await;
    }

    #[tokio::test]
    async fn pinned_model_replay_panic_preserves_intent_in_quarantined_slot() {
        let mut pool = AgentPool::from_slots(vec![None]);
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
                turn_id: "panic-model-turn".into(),
                recoverable_batch: Some(one_event_batch(channel_id, "model-retry")),
                desired_model: Some("runtime-model".into()),
                model_overridden: true,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        started_rx.await.unwrap();
        abort_handle.abort();
        let join_error = pool.join_set.join_next().await.unwrap().unwrap_err();

        let mut queue = EventQueue::new(config::DedupMode::Queue);
        queue.require_agent(channel_id, 0);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut typing_channels = HashMap::new();
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

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
            None,
            None,
        );

        let pending = crash_history[0]
            .pending_model_intent
            .as_ref()
            .expect("pinned panic must retain model intent through replacement");
        assert_eq!(pending.desired_model.as_deref(), Some("runtime-model"));
        assert!(pending.model_overridden);
        assert_eq!(queue.required_agent(channel_id), Some(0));
        assert!(
            crash_history[0].cleanup_unverified,
            "panic loses the typed cleanup owner and must quarantine the slot"
        );
        assert_eq!(
            respawn_tasks.len(),
            0,
            "panic recovery must not spawn a replacement without cleanup proof"
        );
        respawn_tasks.shutdown().await;
    }

    #[tokio::test]
    async fn circuit_open_panic_quarantines_instead_of_scheduling_cooldown_refill() {
        let mut pool = AgentPool::from_slots(vec![None]);
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
                turn_id: "panic-open-circuit".into(),
                recoverable_batch: Some(one_event_batch(channel_id, "model-retry")),
                desired_model: Some("runtime-model".into()),
                model_overridden: true,
                accepted_model_switch: Some(ModelSwitchRequest::new(
                    "runtime-model",
                    "fedcba9876543210fedcba9876543210",
                )),
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        started_rx.await.unwrap();
        abort_handle.abort();
        let join_error = pool.join_set.join_next().await.unwrap().unwrap_err();

        let mut queue = EventQueue::new(config::DedupMode::Queue);
        queue.require_agent(channel_id, 0);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut typing_channels = HashMap::new();
        let mut crash_history = vec![SlotCircuit::new()];
        let now = std::time::Instant::now();
        crash_history[0].crash_times = vec![now; CIRCUIT_BREAKER_THRESHOLD.saturating_sub(1)];
        let (respawn_tx, mut respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

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
            None,
            None,
        );

        assert!(crash_history[0].cleanup_unverified);
        assert!(
            crash_history[0].blocks_supervisor_exit(),
            "quarantine must keep the current process alive and degraded"
        );
        assert!(
            respawn_rx.try_recv().is_err(),
            "panic recovery must not publish a synthetic cleanup-complete marker"
        );
        assert_eq!(
            crash_history[0]
                .pending_model_intent
                .as_ref()
                .and_then(|intent| intent.desired_model.as_deref()),
            Some("runtime-model")
        );
        assert!(respawn_tasks.is_empty());
    }

    #[tokio::test]
    async fn exhausted_panic_retry_dead_letters_and_clears_exact_slot_pin() {
        let mut pool = AgentPool::from_slots(vec![None]);
        let channel_id = Uuid::new_v4();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let abort_handle = pool.join_set.spawn(async move {
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        let task_id = abort_handle.id();
        let mut batch = one_event_batch(channel_id, "poison-batch");
        batch.cancel_reason = None;
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                turn_id: "panic-dead-letter".into(),
                recoverable_batch: Some(batch),
                desired_model: Some("runtime-model".into()),
                model_overridden: true,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        started_rx.await.unwrap();
        abort_handle.abort();
        let join_error = pool.join_set.join_next().await.unwrap().unwrap_err();

        let mut queue = EventQueue::new(config::DedupMode::Queue);
        queue.require_agent(channel_id, 0);
        queue.set_retry_count_for_test(channel_id, crate::queue::MAX_RETRIES);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut typing_channels = HashMap::new();
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

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
            None,
            None,
        );

        assert_eq!(
            queue.required_agent(channel_id),
            None,
            "dead-lettered work must not leave an orphan exact-slot requirement"
        );
        assert!(!queue.has_flushable_work());
        respawn_tasks.shutdown().await;
    }

    #[tokio::test]
    async fn exhausted_panic_retry_preserves_exact_slot_pin_for_queued_residue() {
        let channel_id = Uuid::new_v4();
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        queue.require_agent(channel_id, 0);
        for index in 0..=crate::queue::MAX_BATCH_EVENTS {
            queue.push(crate::queue::QueuedEvent {
                channel_id,
                event: EventBuilder::new(Kind::Custom(9), format!("dependent-{index}"))
                    .sign_with_keys(&Keys::generate())
                    .unwrap(),
                received_at: std::time::Instant::now(),
                prompt_tag: "test".into(),
            });
        }
        let poison_batch = queue.flush_next().expect("first bounded batch");
        assert_eq!(poison_batch.events.len(), crate::queue::MAX_BATCH_EVENTS);
        assert_eq!(queue.queued_event_count(&channel_id), 1);
        queue.set_retry_count_for_test(channel_id, crate::queue::MAX_RETRIES);

        let mut pool = AgentPool::from_slots(vec![None]);
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
                turn_id: "panic-dead-letter-with-residue".into(),
                recoverable_batch: Some(poison_batch),
                desired_model: Some("runtime-model".into()),
                model_overridden: true,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        started_rx.await.unwrap();
        abort_handle.abort();
        let join_error = pool.join_set.join_next().await.unwrap().unwrap_err();

        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut typing_channels = HashMap::new();
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

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
            None,
            None,
        );

        assert_eq!(
            queue.required_agent(channel_id),
            Some(0),
            "terminal disposal must preserve exact-slot affinity for dependent residue"
        );
        assert_eq!(
            queue.queued_event_count(&channel_id),
            1,
            "the batch boundary must leave dependent residue queued"
        );
        respawn_tasks.shutdown().await;
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

    #[tokio::test]
    async fn optional_close_recycle_respawns_without_crash_penalty() {
        let disposition = outcome_disposition_for(PromptOutcome::SessionRecycleRequired).await;

        assert_eq!(
            disposition.live_agents, 0,
            "the adapter must leave service until its process group is replaced"
        );
        assert_eq!(
            disposition.respawn_tasks, 1,
            "optional-close fallback must start one intentional replacement"
        );
        assert_eq!(
            disposition.turn_errors, 0,
            "a negotiated compatibility fallback is not an operator error"
        );
        assert_eq!(
            disposition.recent_crashes, 0,
            "intentional replacement must not consume crash budget"
        );
        assert!(
            !disposition.circuit_open,
            "normal retirements must never open the crash circuit"
        );
    }

    #[tokio::test]
    async fn optional_close_recycle_preserves_live_model_override() {
        let mut agent = dummy_agent(0).await;
        agent.desired_model = Some("runtime-model".into());
        agent.model_overridden = true;
        let config = test_config();
        let mut slot = SlotCircuit::new();
        let (respawn_tx, mut respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        assert!(spawn_recycle_task(
            agent,
            &config,
            &mut slot,
            &respawn_tx,
            &mut respawn_tasks,
            None,
        ));
        let result = tokio::time::timeout(Duration::from_secs(5), respawn_rx.recv())
            .await
            .expect("intentional recycle must report its spawn result")
            .expect("respawn result channel must stay open");

        assert_eq!(result.desired_model.as_deref(), Some("runtime-model"));
        assert!(
            result.model_overridden,
            "busy SwitchModel intent must survive the compatibility recycle"
        );
        assert!(
            slot.crash_times.is_empty() && slot.open_until.is_none(),
            "intentional recycle must not mutate the crash circuit"
        );
    }

    #[tokio::test]
    async fn optional_close_recycle_reserves_per_slot_rate_budget() {
        let agent = dummy_agent(0).await;
        let config = test_config();
        let mut slot = SlotCircuit::new();
        let (respawn_tx, _respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let started = std::time::Instant::now();

        assert!(spawn_recycle_task(
            agent,
            &config,
            &mut slot,
            &respawn_tx,
            &mut respawn_tasks,
            None,
        ));
        let next = slot
            .next_recycle_not_before
            .expect("recycle must reserve the next per-slot budget");
        assert!(
            next >= started + RECYCLE_MIN_INTERVAL,
            "the next negotiated recycle must be delayed by the minimum interval"
        );
        assert!(slot.crash_times.is_empty() && slot.open_until.is_none());
        respawn_tasks.shutdown().await;
    }

    #[tokio::test]
    async fn circuit_open_still_schedules_guaranteed_old_adapter_cleanup() {
        let agent = dummy_agent(0).await;
        let config = test_config();
        let mut slot = SlotCircuit::new();
        slot.open_until = Some(std::time::Instant::now() + CIRCUIT_BREAKER_COOLDOWN);
        let (respawn_tx, mut respawn_rx) = mpsc::channel(1);
        let mut respawn_tasks = tokio::task::JoinSet::new();

        assert!(
            spawn_respawn_task(
                agent,
                &config,
                &mut slot,
                &respawn_tx,
                &mut respawn_tasks,
                None,
            ),
            "an open circuit may suppress replacement, never bounded shutdown"
        );
        assert!(slot.respawn_in_flight);
        let result = tokio::time::timeout(Duration::from_secs(2), respawn_rx.recv())
            .await
            .expect("cleanup completion must wake the supervisor")
            .expect("cleanup result channel must stay open");
        assert!(
            result.result.is_ok(),
            "cleanup-only completion is not a replacement spawn failure"
        );
        respawn_tasks
            .join_next()
            .await
            .expect("cleanup task must be tracked")
            .expect("cleanup task must not panic");
    }

    #[tokio::test]
    async fn every_session_close_failure_poison_class_respawns_the_adapter() {
        let cases = [
            (
                "unsupported",
                AcpError::AgentError {
                    code: -32601,
                    message: "method not found".into(),
                },
            ),
            (
                "adapter error",
                AcpError::AgentError {
                    code: -32000,
                    message: "close rejected".into(),
                },
            ),
            (
                "timeout",
                AcpError::Timeout(std::time::Duration::from_secs(5)),
            ),
        ];

        for (name, error) in cases {
            let disposition =
                outcome_disposition_for(PromptOutcome::SessionCloseFailed(error)).await;
            assert_eq!(
                disposition.live_agents, 0,
                "{name}: poisoned adapter must not return to the idle pool"
            );
            assert_eq!(
                disposition.respawn_tasks, 1,
                "{name}: poisoned adapter must enter process-group replacement"
            );
            assert_eq!(
                disposition.turn_errors, 1,
                "{name}: operator feed must record the cleanup failure"
            );
        }
    }

    #[tokio::test]
    async fn session_close_failure_redacts_adapter_controlled_message() {
        let secret = "adapter-secret-that-must-not-enter-observer";
        let disposition =
            outcome_disposition_for(PromptOutcome::SessionCloseFailed(AcpError::AgentError {
                code: -32000,
                message: secret.into(),
            }))
            .await;

        assert_eq!(disposition.turn_error_texts.len(), 1);
        assert!(
            disposition
                .turn_error_texts
                .iter()
                .all(|text| !text.contains(secret)),
            "adapter-controlled error text must not enter the observer feed"
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
                    turn_id: "test-turn-id".to_string(),
                    recoverable_batch: None,
                    desired_model: None,
                    model_overridden: false,
                    accepted_model_switch: None,
                    accepted_drop_control: None,
                    control_tx: None,
                    steer_tx: None,
                },
            );
            let mut queue = EventQueue::new(config::DedupMode::Queue);
            let config = test_config();
            let mut heartbeat_in_flight = false;
            let removed_channels = HashSet::new();
            let mut crash_history = vec![SlotCircuit::new()];
            let (respawn_tx, _respawn_rx) = mpsc::channel(8);
            let mut respawn_tasks = tokio::task::JoinSet::new();
            let observer = ObserverHandle::in_process();
            let result = PromptResult {
                agent,
                source: PromptSource::Channel(Uuid::new_v4()),
                turn_id: "test-turn-id".to_string(),
                outcome,
                batch: None,
                retry_agent_index: None,
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
            FlushBatch {
                channel_id: Uuid::new_v4(),
                events: vec![BatchEvent {
                    event,
                    prompt_tag: "test".into(),
                    received_at: std::time::Instant::now(),
                }],
                cancelled_events: vec![],
                cancel_reason: None,
                occurrence_ids: BatchOccurrenceIds::for_test(1, 0),
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
                    turn_id: "test-turn-id".to_string(),
                    recoverable_batch: None,
                    desired_model: None,
                    model_overridden: false,
                    accepted_model_switch: None,
                    accepted_drop_control: None,
                    control_tx: None,
                    steer_tx: None,
                },
            );
            let mut queue = EventQueue::new(config::DedupMode::Queue);
            let config = test_config();
            let mut heartbeat_in_flight = false;
            let removed_channels = HashSet::new();
            let mut crash_history = vec![SlotCircuit::new()];
            let (respawn_tx, _respawn_rx) = mpsc::channel(8);
            let mut respawn_tasks = tokio::task::JoinSet::new();
            let result = PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "test-turn-id".to_string(),
                outcome,
                batch: Some(batch),
                retry_agent_index: None,
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
                events: vec![BatchEvent {
                    event,
                    prompt_tag: "test".into(),
                    received_at: std::time::Instant::now(),
                }],
                cancelled_events: vec![],
                cancel_reason: None,
                occurrence_ids: BatchOccurrenceIds::for_test(1, 0),
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
                    turn_id: "test-turn-id".to_string(),
                    recoverable_batch: None,
                    desired_model: None,
                    model_overridden: false,
                    accepted_model_switch: None,
                    accepted_drop_control: None,
                    control_tx: None,
                    steer_tx: None,
                },
            );
            let mut queue = EventQueue::new(config::DedupMode::Queue);
            let config = test_config();
            let mut heartbeat_in_flight = false;
            let removed_channels = HashSet::new();
            let mut crash_history = vec![SlotCircuit::new()];
            let (respawn_tx, _respawn_rx) = mpsc::channel(8);
            let mut respawn_tasks = tokio::task::JoinSet::new();
            let result = PromptResult {
                agent,
                source: PromptSource::Channel(channel_id),
                turn_id: "test-turn-id".to_string(),
                outcome,
                batch: Some(batch),
                retry_agent_index: None,
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
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let observer = ObserverHandle::in_process();
        let batch = FlushBatch {
            channel_id,
            events: vec![BatchEvent {
                event: EventBuilder::new(Kind::Custom(9), "test")
                    .sign_with_keys(&Keys::generate())
                    .unwrap(),
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
            occurrence_ids: BatchOccurrenceIds::for_test(1, 0),
        };
        let result = PromptResult {
            agent,
            source: PromptSource::Channel(channel_id),
            turn_id: "test-turn-id".to_string(),
            outcome: PromptOutcome::Timeout(TimeoutKind::Hard {
                recently_active: true,
            }),
            batch: Some(batch),
            retry_agent_index: None,
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
        queue.require_agent(channel_id, 0);

        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: None,
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let observer = ObserverHandle::in_process();
        let batch = FlushBatch {
            channel_id,
            events: vec![BatchEvent {
                event: EventBuilder::new(Kind::Custom(9), "final-attempt")
                    .sign_with_keys(&Keys::generate())
                    .unwrap(),
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
            occurrence_ids: BatchOccurrenceIds::for_test(1, 0),
        };
        let result = PromptResult {
            agent,
            source: PromptSource::Channel(channel_id),
            turn_id: "test-turn-id".to_string(),
            outcome: PromptOutcome::Timeout(TimeoutKind::Hard {
                recently_active: true,
            }),
            batch: Some(batch),
            retry_agent_index: None,
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
        assert_eq!(
            queue.required_agent(channel_id),
            None,
            "terminal dead-letter must release its exact-slot requirement"
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
            events: vec![BatchEvent {
                event: original_event.clone(),
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: Some(CancelReason::Steer),
            occurrence_ids: BatchOccurrenceIds::for_test(1, 0),
        };

        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: None,
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
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
        let mut crash_history = vec![SlotCircuit::new()];
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
            retry_agent_index: None,
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
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();
        let mut crash_history = vec![SlotCircuit::new()];
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
            retry_agent_index: None,
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
    async fn malformed_json_response_poisons_adapter_instead_of_reusing_it() {
        let json_error = serde_json::from_str::<serde_json::Value>("{")
            .expect_err("fixture must be malformed JSON");
        let disposition =
            outcome_disposition_for(PromptOutcome::Error(AcpError::Json(json_error))).await;
        assert_eq!(disposition.live_agents, 0);
        assert_eq!(
            disposition.respawn_tasks, 1,
            "JSON framing corruption must replace the adapter"
        );
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
            events: vec![BatchEvent {
                event,
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
            occurrence_ids: BatchOccurrenceIds::for_test(1, 0),
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
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = std::collections::HashSet::new();
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let result = PromptResult {
            agent,
            source: PromptSource::Channel(channel_id),
            turn_id: "test-turn-id".to_string(),
            outcome: PromptOutcome::Error(auth_error),
            batch: Some(batch),
            retry_agent_index: None,
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
            events: vec![BatchEvent {
                event,
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
            occurrence_ids: BatchOccurrenceIds::for_test(1, 0),
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
                turn_id: "test-turn-id".to_string(),
                recoverable_batch: None,
                desired_model: None,
                model_overridden: false,
                accepted_model_switch: None,
                accepted_drop_control: None,
                control_tx: None,
                steer_tx: None,
            },
        );
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let config = test_config();
        let mut heartbeat_in_flight = false;
        let removed_channels = std::collections::HashSet::new();
        let mut crash_history = vec![SlotCircuit::new()];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let result = PromptResult {
            agent,
            source: PromptSource::Channel(channel_id),
            turn_id: "test-turn-id".to_string(),
            outcome: PromptOutcome::Error(usage_error),
            batch: Some(batch),
            retry_agent_index: None,
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
