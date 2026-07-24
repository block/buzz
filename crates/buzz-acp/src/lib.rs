#![deny(unsafe_code)]

mod acp;
mod config;
mod engram_fetch;
mod filter;
mod ledger;
mod observer;
mod pool;
mod pool_lifecycle;
mod queue;
mod relay;
mod setup_mode;
mod usage;

pub use usage::TurnUsage;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
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
use ledger::Ledger;
use nostr::{PublicKey, ToBech32};
use pool::{
    AgentPool, ControlSignal, IdleSwitchResult, OwnedAgent, PromptContext, PromptOutcome,
    PromptResult, PromptSource, SessionState, TimeoutKind,
};
use pool_lifecycle::PoolLifecycle;
use queue::{BatchDisposition, CancelReason, EventQueue, FlushBatch, QueuedEvent, ThreadTags};
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

fn handle_relay_observer_control_event(
    keys: &nostr::Keys,
    event: nostr::Event,
    pool: &mut AgentPool,
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

    let command_type = payload.get("type").and_then(|value| value.as_str());
    match command_type {
        Some("cancel_turn") => {
            handle_cancel_turn_control(&payload, pool, observer);
        }
        Some("switch_model") => {
            handle_switch_model_control(&payload, pool, observer);
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

    let fired = signal_in_flight_task(pool, channel_id, ControlSignal::Cancel);
    let status = if fired { "sent" } else { "no_active_turn" };
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
/// Idle path: validate against the cached catalog *before* invalidating
/// (pre-cancel guard), then set `desired_model` + invalidate. The override
/// takes visible effect on the agent's next turn.
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

    // A turn is in flight for this channel iff a task_map entry exists. The
    // agent is moved out of the pool during a turn, so the control oneshot is
    // the only reachable lever; an idle channel has no such entry.
    let turn_in_flight = pool
        .task_map()
        .values()
        .any(|m| m.channel_id == Some(channel_id));

    let status = if turn_in_flight {
        // Busy path: deliver over the oneshot. `false` means the oneshot was
        // already consumed this turn (a prior cancel/interrupt) — the turn is
        // already ending, so the switch cannot land on it.
        if signal_in_flight_task(
            pool,
            channel_id,
            ControlSignal::SwitchModel(model_id.to_string()),
        ) {
            "sent"
        } else {
            "turn_ending"
        }
    } else {
        // Idle path: validate against the cached catalog before invalidating.
        match pool.switch_idle_agent_model(channel_id, model_id) {
            IdleSwitchResult::Switched => "switched",
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
            serde_json::json!({
                "type": "switch_model",
                "status": status,
                "modelId": model_id,
            }),
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
    /// Tuple: (initialized client, protocol version, supports_goose_steer).
    /// The third element is always `true` — the supervisor uses
    /// try-and-tolerate for the steer extension.
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
    event_id: String,
    /// `Ok` if the read loop sent any of the locked `SteerAck` variants.
    /// `Err` if the oneshot was dropped without a send — should not happen
    /// under the current read-loop drains, but if it ever does the main
    /// loop treats it as `PromptCompletedNeutral` (release withheld, no
    /// fallback signal) to avoid leaking the withheld event.
    ack: std::result::Result<pool::SteerAck, tokio::sync::oneshot::error::RecvError>,
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

pub fn run() -> Result<()> {
    config::propagate_legacy_env_vars();
    tokio_main()
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
    }

    // Durable pending-turn ledger (auto-resume after restart). Disabled
    // outright (`Ledger::disabled()`, no disk I/O) when the flag is off;
    // otherwise loaded now so boot recovery can stage it before the main
    // loop starts. See `boot_recover` below and
    // `PLANS/AGENT_AUTO_RESUME_LEDGER.md`.
    let agent_pubkey_hex = config.keys.public_key().to_hex();
    let (mut ledger, staged_ledger) = if config.resume_on_restart {
        let state_dir = config.state_dir.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".buzz-acp")
        });
        Ledger::load(
            &state_dir,
            &agent_pubkey_hex,
            &config.relay_url,
            config.resume_ttl_secs,
        )
    } else {
        (Ledger::disabled(), ledger::StagedLedger::default())
    };

    // Boot recovery: membership-gate → chunked REST fetch (reconciled
    // per-event) → import in admission_seq order (with cap promotion) →
    // suppression set → unresolved barrier registration → single commit.
    // No-ops end to end when the ledger is disabled or the stage is empty.
    // See `boot_recover` and `PLANS/AGENT_AUTO_RESUME_LEDGER.md` §"Boot
    // recovery".
    let mut recovered_suppression: HashSet<String> = HashSet::new();
    if ledger.is_enabled() {
        boot_recover(
            &mut queue,
            &mut ledger,
            staged_ledger,
            &channel_ids.iter().copied().collect(),
            &relay.rest_client(),
            &mut recovered_suppression,
        )
        .await;
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
        SteerAck(SteerAckEvent),
        Wake(u32, Result<AgentPool, String>),
    }

    // Boot-recovery wake: dispatch any recovered work immediately so a
    // quiet harness does not wait for the first 30s maintenance cycle.
    for (channel_id, thread_tags) in dispatch_pending(&mut pool, &mut queue, &mut ledger, &ctx) {
        typing_channels.insert(channel_id, thread_tags);
    }

    loop {
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
            //
            // has_flushable_work()'s expiry loop can dirty a channel even
            // when it returns false, so sync unconditionally.
            if queue.has_flushable_work() {
                for (channel_id, thread_tags) in
                    dispatch_pending(&mut pool, &mut queue, &mut ledger, &ctx)
                {
                    typing_channels.insert(channel_id, thread_tags);
                }
            } else {
                sync_dirty(&mut queue, &mut ledger);
            }
        }

        let mut respawn_collected = false;
        while let Ok(rr) = respawn_rx.try_recv() {
            crash_history[rr.index].respawn_in_flight = false;
            match rr.result {
                Ok((acp, protocol_version, agent_name)) => {
                    let agent = OwnedAgent {
                        index: rr.index,
                        acp,
                        state: SessionState::default(),
                        model_capabilities: None,
                        desired_model: config.model.clone(),
                        model_overridden: false,
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
                dispatch_pending(&mut pool, &mut queue, &mut ledger, &ctx)
            {
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
                                handle_relay_observer_control_event(&config.keys, event, &mut pool, observer.as_ref(), owner_hex);
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
                                    // Drain queued events and invalidate sessions for the
                                    // removed channel. Events already in-flight will
                                    // complete normally (the relay may reject actions if
                                    // the agent lost access).
                                    let drained_ids = queue.drain_channel(ch);
                                    let invalidated = if pool_ready {
                                        pool.invalidate_channel_sessions(ch)
                                    } else {
                                        0
                                    };
                                    // Purge durable live + unresolved records together —
                                    // re-adding the channel later must not resurrect
                                    // discarded work (P3-F3 invalidation exit).
                                    ledger.invalidate_channel(ch);
                                    // Track removed channels so checked-out agents get
                                    // their sessions stripped when they return to the pool.
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
                                    if !drained_ids.is_empty() || invalidated > 0 {
                                        tracing::info!(
                                            channel_id = %ch,
                                            drained = drained_ids.len(),
                                            invalidated,
                                            "cleaned up after membership removal"
                                        );
                                    }
                                    // drain_channel marked `ch` dirty; this branch
                                    // `continue`s below rather than falling through
                                    // to a `dispatch_pending` call, so sync here.
                                    sync_dirty(&mut queue, &mut ledger);
                                }
                                continue;
                            }

                            if config.ignore_self && buzz_event.event.pubkey.to_hex() == pubkey_hex {
                                tracing::debug!(channel_id = %buzz_event.channel_id, "dropping self-authored event");
                                continue;
                            }

                            // Check: kind:9, content "!shutdown", from owner, mentions THIS agent.
                            let is_shutdown = is_owner_control_command(
                                &buzz_event.event,
                                kind_u32,
                                "!shutdown",
                                &pubkey_hex,
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
                            );
                            if is_cancel {
                                if let Some(owner) = owner_cache.get() {
                                    if buzz_event.event.pubkey.to_hex() == *owner {
                                        let fired = signal_in_flight_task(
                                            &mut pool,
                                            buzz_event.channel_id,
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
                            );
                            if is_rotate {
                                if let Some(owner) = owner_cache.get() {
                                    if buzz_event.event.pubkey.to_hex() == *owner {
                                        let fired = signal_in_flight_task(
                                            &mut pool,
                                            buzz_event.channel_id,
                                            ControlSignal::Rotate,
                                        );
                                        if fired {
                                            tracing::info!(
                                                channel_id = %buzz_event.channel_id,
                                                "!rotate received — cancelling in-flight turn and rotating session"
                                            );
                                        } else {
                                            let invalidated = pool.invalidate_channel_sessions(buzz_event.channel_id);
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

                            // R6-F2: recovered-suppression seam. If this
                            // event was boot-recovered (its id is in the
                            // suppression set), it is already in the queue
                            // via import_recovered — the relay replay is a
                            // duplicate. But if the event was unresolved
                            // (not fetched during boot), the live arrival
                            // is the resolution: admit it at its original
                            // seq position and clear the unresolved record.
                            // Either way, skip try_native_steer — recovered
                            // admissions wait for recovery-framed batch
                            // dispatch, not mid-turn steering. See
                            // `admit_live_event`.
                            let (accepted, skip_steer) = admit_live_event(
                                &mut queue,
                                &mut ledger,
                                &mut recovered_suppression,
                                buzz_event.channel_id,
                                buzz_event.event,
                                prompt_tag,
                            );

                            if accepted {
                                let rc = ctx.rest_client.clone();
                                let eid = event_id_hex.clone();
                                tokio::spawn(async move {
                                    pool::reaction_add(&rc, &eid, "👀").await;
                                });
                            }
                            if accepted && !skip_steer && queue.is_channel_in_flight(buzz_event.channel_id) {
                                let signal = mode_gate_signal(
                                    config.multiple_event_handling,
                                    &author_hex,
                                    owner_cache.get(),
                                );
                                if let Some(signal) = signal {
                                    let native_attempted = matches!(signal, ControlSignal::Steer)
                                        && try_native_steer(
                                            &mut pool,
                                            &mut queue,
                                            buzz_event.channel_id,
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
                            if pool_ready {
                                for (channel_id, thread_tags) in
                                    dispatch_pending(&mut pool, &mut queue, &mut ledger, &ctx)
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
                    match heartbeat.as_mut() {
                        Some(hb) => hb.tick().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let _ = result_rx;
                    if !pool_ready {
                        tracing::debug!("heartbeat_skipped_pool_not_ready");
                    } else {
                        // has_flushable_work()'s expiry loop can dirty a channel
                        // even when it returns false (e.g. it auto-released a
                        // stuck in-flight channel that still isn't flushable for
                        // some other reason) — sync unconditionally so that
                        // dirty channel is never left unsynced.
                        let flushable = queue.has_flushable_work();
                        if flushable {
                            tracing::debug!("heartbeat_skipped_events");
                            for (channel_id, thread_tags) in
                                dispatch_pending(&mut pool, &mut queue, &mut ledger, &ctx)
                            {
                                typing_channels.insert(channel_id, thread_tags);
                            }
                        } else {
                            sync_dirty(&mut queue, &mut ledger);
                            if pool.any_idle() {
                                dispatch_heartbeat(&mut pool, &ctx, &mut heartbeat_in_flight);
                            } else {
                                tracing::debug!("heartbeat_skipped_busy");
                            }
                        }
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
                // Barrier-deadline timer arm (rev 6.1): fires when the
                // earliest unresolved barrier expires, even when all
                // external inputs are silent. Without this, held suffix
                // events on quiet channels would never dispatch.
                _ = async {
                    match queue.next_unresolved_barrier_deadline() {
                        Some(deadline) => {
                            let tokio_deadline = tokio::time::Instant::from_std(deadline);
                            tokio::time::sleep_until(tokio_deadline).await;
                        }
                        None => std::future::pending().await,
                    }
                } => {
                    let _ = result_rx;
                    let expired = queue.expire_due_unresolved_barriers(std::time::Instant::now());
                    if !expired.is_empty() {
                        tracing::info!(
                            channels = ?expired,
                            "unresolved barrier deadline expired — releasing held suffix"
                        );
                        sync_dirty(&mut queue, &mut ledger);
                        for (channel_id, thread_tags) in
                            dispatch_pending(&mut pool, &mut queue, &mut ledger, &ctx)
                        {
                            typing_channels.insert(channel_id, thread_tags);
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
                    // Sync before exit: complete_batch dirtied the queue but
                    // the exit bypasses dispatch_pending's sync call (P3-F2).
                    sync_dirty(&mut queue, &mut ledger);
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
                    // Sync before exit: classify dirtied the queue but the
                    // exit bypasses dispatch_pending's sync call (P3-F2).
                    sync_dirty(&mut queue, &mut ledger);
                    break;
                }
                for (channel_id, thread_tags) in
                    dispatch_pending(&mut pool, &mut queue, &mut ledger, &ctx)
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
                    // Sync before exit: complete_batch dirtied the queue but
                    // the exit bypasses dispatch_pending's sync call (P3-F2).
                    sync_dirty(&mut queue, &mut ledger);
                    break;
                }
                for (channel_id, thread_tags) in
                    dispatch_pending(&mut pool, &mut queue, &mut ledger, &ctx)
                {
                    typing_channels.insert(channel_id, thread_tags);
                }
            }
            Some(PoolEvent::SteerAck(SteerAckEvent {
                channel_id,
                event_id,
                ack,
            })) => {
                // Goose-native steer attempt resolved. Locked semantics
                // (Eva + Max + Perci, unanimous on Option X):
                //
                //   Success
                //     The agent received the steer via the non-cancelling
                //     path. Drop the withheld event so normal dispatch
                //     never redelivers it.
                //
                //   Err(_) where the write never landed (Transport /
                //   ExpectedRunIdMissing):
                //     Delivery state of the underlying message is "never
                //     attempted on the wire". Release withheld back to the
                //     queue front AND issue the cancel+merge fallback so
                //     the message still reaches the agent.
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
                    // Transport / ExpectedRunIdMissing: write never landed.
                    // Release and fire the cancel+merge fallback so the
                    // message still reaches the agent.
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
                    queue.remove_event(channel_id, &event_id);
                }
                if release_withheld {
                    queue.release_native_steer(channel_id, &event_id);
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
                for (channel_id, thread_tags) in
                    dispatch_pending(&mut pool, &mut queue, &mut ledger, &ctx)
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
                            dispatch_pending(&mut pool, &mut queue, &mut ledger, &ctx)
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
    drain_shutdown_grace(
        &mut pool,
        &mut queue,
        &mut ledger,
        &config,
        &removed_channels,
        Some(&ctx.rest_client),
        grace,
    )
    .await;
    // Drain any remaining results that arrived after join_set drained but
    // before tasks were aborted. Same R5-F2 classification as above.
    while let Ok(mut pr) = pool.result_rx_try_recv() {
        let idx = pr.agent.index;
        classify_and_complete_batch(
            &mut queue,
            &config,
            &mut pr,
            &removed_channels,
            Some(&ctx.rest_client),
        );
        sync_dirty(&mut queue, &mut ledger);
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

    // Graceful relay shutdown — sends WebSocket close frame and waits up to 5s
    // for the background task to finish, rather than aborting immediately (#40).
    relay.shutdown().await;

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

// ── admit_live_event ─────────────────────────────────────────────────────────

/// Admit a live relay event into the queue, resolving it against boot
/// recovery's suppression set and unresolved barrier first (R6-F2).
///
/// Returns `(accepted, skip_steer)`: `accepted` is whether the event landed
/// in the queue (mirrors `EventQueue::push`'s admission result on the normal
/// path); `skip_steer` tells the caller whether to bypass `try_native_steer`
/// for this event — recovered admissions (whether a suppressed duplicate or
/// a just-resolved unresolved record) wait for recovery-framed batch
/// dispatch, not mid-turn steering.
///
/// This is the exact decision the main loop's relay-event arm makes inline;
/// extracted so tests can drive it directly instead of mirroring it.
fn admit_live_event(
    queue: &mut EventQueue,
    ledger: &mut Ledger,
    recovered_suppression: &mut HashSet<String>,
    channel_id: uuid::Uuid,
    event: nostr::Event,
    prompt_tag: String,
) -> (bool, bool) {
    let event_id_hex = event.id.to_hex();
    let is_recovered_suppressed = recovered_suppression.remove(&event_id_hex);
    if is_recovered_suppressed {
        // Already in the queue from boot import — suppress the duplicate
        // live push entirely.
        (false, true)
    } else if let Some(unresolved) = ledger.find_unresolved(channel_id, &event_id_hex) {
        // Unresolved record resolving: build a recovered QueuedEvent with
        // the ledger's original seq/timestamp/cap_exempt/prompt_tag.
        // The persisted `prompt_tag` is used rather than the live match's
        // tag — a config or rule-priority change between admission and
        // restart must not alter recovery framing (P3-F4).
        let recovered = QueuedEvent::from_recovered(
            channel_id,
            event,
            unresolved.prompt_tag.clone(),
            unresolved.admission_seq,
            unresolved.enqueued_at_unix,
            unresolved.cap_exempt,
        );
        queue.admit_recovered(channel_id, recovered);
        ledger.resolve_unresolved(channel_id, &event_id_hex);
        sync_dirty(queue, ledger);
        (true, true)
    } else {
        let ok = queue.push(QueuedEvent::new(
            channel_id,
            event,
            std::time::Instant::now(),
            prompt_tag,
        ));
        // While the channel has an active unresolved barrier the steer
        // side-door is suppressed: the fresh live event queues normally and
        // waits behind the barrier hole rather than bypassing it (P3-F1).
        let skip_steer = queue.has_active_unresolved_barrier(channel_id);
        (ok, skip_steer)
    }
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
fn signal_in_flight_task(
    pool: &mut AgentPool,
    channel_id: uuid::Uuid,
    mode: ControlSignal,
) -> bool {
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
        // Synthetic — built only to render the steer body via
        // `format_event_block`; never persisted to the queue or ledger, so
        // the recovery fields are inert placeholders.
        admission_seq: 0,
        enqueued_at_unix: 0,
        restart_recovery: false,
        cap_exempt: false,
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
            let withheld = queue.mark_native_steer_pending(channel_id, &event_id_hex);
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

// ── boot_recover ──────────────────────────────────────────────────────────────

/// Maximum event ids fetched per REST `/query` chunk during boot recovery.
/// One `Filter` with `ids(chunk)` per request — the relay is queried by
/// id-set, not per-id (see `PLANS/AGENT_AUTO_RESUME_LEDGER.md` R5-F5).
const BOOT_RECOVERY_CHUNK_SIZE: usize = 100;

/// Overall wall-clock budget for the boot-recovery REST fetch phase,
/// regardless of ledger cardinality.
const BOOT_RECOVERY_FETCH_DEADLINE: Duration = Duration::from_secs(60);

/// Unresolved-barrier timeout measured from boot commit (after fetch
/// reconciliation completes), not from the start of the fetch phase.
const BOOT_RECOVERY_BARRIER_DEADLINE: Duration = Duration::from_secs(60);

/// Boot recovery (rev 6.1): stage → membership gate → TTL (already applied
/// by `Ledger::load`) → chunked id-set fetch under one global deadline,
/// each chunk reconciled per-event → `import_recovered` in global
/// `admission_seq` order (cap promotion) → suppression set → unresolved
/// barrier registration → single commit merging unresolved. See
/// `PLANS/AGENT_AUTO_RESUME_LEDGER.md` §"Boot recovery" for the full
/// design and its numbered edge cases.
///
/// A no-op (immediate return) when the staged ledger has nothing to
/// recover. Never blocks boot past `BOOT_RECOVERY_FETCH_DEADLINE`: any trigger
/// not fetched by then is retained as unresolved rather than dropped.
async fn boot_recover(
    queue: &mut EventQueue,
    ledger: &mut Ledger,
    staged: ledger::StagedLedger,
    live_channels: &HashSet<Uuid>,
    rest: &relay::RestClient,
    recovered_suppression: &mut HashSet<String>,
) {
    if staged.channels.is_empty() {
        return;
    }

    // Membership gate (fixes F5): a ledger channel this boot didn't
    // discover/subscribe to is dropped wholesale — no speculative
    // re-subscription as validation.
    let mut gated: Vec<(Uuid, ledger::LedgerRecord)> = Vec::new();
    let mut dropped_channels = 0usize;
    for (channel_id, records) in staged.channels {
        if live_channels.contains(&channel_id) {
            gated.extend(records.into_iter().map(|r| (channel_id, r)));
        } else {
            dropped_channels += 1;
        }
    }
    if dropped_channels > 0 {
        tracing::info!(
            dropped_channels,
            "boot recovery: dropped ledger channel(s) no longer in this boot's membership"
        );
    }
    let fetch_deadline = tokio::time::Instant::now() + BOOT_RECOVERY_FETCH_DEADLINE;
    let all_ids: Vec<String> = gated.iter().map(|(_, r)| r.event_id.clone()).collect();
    let by_id: HashMap<&str, &(Uuid, ledger::LedgerRecord)> = gated
        .iter()
        .map(|entry| (entry.1.event_id.as_str(), entry))
        .collect();

    let mut fetched: HashMap<String, nostr::Event> = HashMap::new();
    'chunks: for chunk in all_ids.chunks(BOOT_RECOVERY_CHUNK_SIZE) {
        if tokio::time::Instant::now() >= fetch_deadline {
            tracing::warn!("boot recovery: global fetch deadline reached — remaining triggers unresolved-retained");
            break 'chunks;
        }
        let ids: Vec<nostr::EventId> = chunk
            .iter()
            .filter_map(|id| nostr::EventId::from_hex(id).ok())
            .collect();
        if ids.is_empty() {
            continue;
        }
        let filter = nostr::Filter::new().ids(ids);
        let remaining = fetch_deadline.saturating_duration_since(tokio::time::Instant::now());
        let query = tokio::time::timeout(remaining, rest.query(std::slice::from_ref(&filter)));
        let response = match query.await {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                tracing::warn!(
                    "boot recovery: chunk fetch failed: {e} — chunk unresolved-retained"
                );
                continue;
            }
            Err(_) => {
                tracing::warn!("boot recovery: fetch deadline reached mid-chunk — remaining triggers unresolved-retained");
                break 'chunks;
            }
        };
        let Some(events) = response.as_array() else {
            tracing::warn!(
                "boot recovery: chunk response was not a JSON array — chunk unresolved-retained"
            );
            continue;
        };
        let requested: HashSet<&str> = chunk.iter().map(String::as_str).collect();
        for raw in events {
            // Responses are reconciled, never trusted (R6-F5): deserialize,
            // verify signature/id, require id in the requested chunk and
            // its `h` tag to match the ledger record's channel.
            let event = match serde_json::from_value::<nostr::Event>(raw.clone()) {
                Ok(ev) => ev,
                Err(e) => {
                    tracing::warn!(
                        "boot recovery: malformed event in chunk response: {e} — skipped"
                    );
                    continue;
                }
            };
            if event.verify().is_err() {
                tracing::warn!(event_id = %event.id.to_hex(), "boot recovery: event failed signature verification — skipped");
                continue;
            }
            let id_hex = event.id.to_hex();
            if !requested.contains(id_hex.as_str()) {
                tracing::warn!(event_id = %id_hex, "boot recovery: event id not in requested chunk — skipped");
                continue;
            }
            let Some((channel_id, _)) = by_id.get(id_hex.as_str()).copied() else {
                continue;
            };
            let ch_str = channel_id.to_string();
            let h_tag_matches = event.tags.iter().any(|tag| {
                let v = tag.as_slice();
                v.len() >= 2 && v[0] == "h" && v[1] == ch_str
            });
            if !h_tag_matches {
                tracing::warn!(event_id = %id_hex, channel_id = %channel_id, "boot recovery: event's h-tag does not match ledger record's channel — skipped");
                continue;
            }
            fetched.entry(id_hex).or_insert(event);
        }
    }

    // Epoch promotion: compute per channel from ALL gated records before
    // the fetch split. If no record in the channel's full snapshot is
    // exempt, every record (fetched and unresolved) promotes to exempt.
    let mut promote_channels: HashSet<Uuid> = HashSet::new();
    {
        let mut has_exempt: HashMap<Uuid, bool> = HashMap::new();
        for (channel_id, record) in &gated {
            let entry = has_exempt.entry(*channel_id).or_insert(false);
            if record.cap_exempt {
                *entry = true;
            }
        }
        for (channel_id, exempt) in has_exempt {
            if !exempt {
                promote_channels.insert(channel_id);
            }
        }
    }

    // Restore fetched events per channel via import_recovered, in global
    // admission_seq order; unfetched ids stay unresolved-retained. The
    // settled cap class comes from the whole-snapshot promotion above.
    let mut per_channel: HashMap<Uuid, Vec<QueuedEvent>> = HashMap::new();
    let mut max_seq: u64 = 0;
    for (channel_id, mut record) in gated {
        max_seq = max_seq.max(record.admission_seq);
        if promote_channels.contains(&channel_id) {
            record.cap_exempt = true;
        }
        match fetched.remove(&record.event_id) {
            Some(event) => {
                recovered_suppression.insert(record.event_id.clone());
                per_channel
                    .entry(channel_id)
                    .or_default()
                    .push(QueuedEvent::from_recovered(
                        channel_id,
                        event,
                        record.prompt_tag,
                        record.admission_seq,
                        record.enqueued_at_unix,
                        record.cap_exempt,
                    ));
            }
            None => ledger.add_unresolved(channel_id, record),
        }
    }
    queue.set_next_admission_seq(max_seq);
    for (channel_id, events) in per_channel {
        queue.import_recovered(channel_id, events);
    }

    // Register the unresolved ordering barrier per channel (R6-F1) before
    // the single commit, so the queue's flush gate is armed from boot.
    // The barrier deadline starts NOW (after fetch reconciliation), not
    // from the start of the fetch phase — a slow bridge must not consume
    // the resolution window.
    let barrier_deadline = std::time::Instant::now() + BOOT_RECOVERY_BARRIER_DEADLINE;
    for channel_id in ledger.unresolved_channels() {
        queue.set_unresolved_barrier(
            channel_id,
            ledger.unresolved_seqs(channel_id),
            barrier_deadline,
        );
    }

    // Single transactional commit (P2-F4): live ∪ unresolved, computed
    // after every import above. Resumes normal per-mutation sync from here.
    let mut live: HashMap<Uuid, Vec<queue::RecoverableTrigger>> = HashMap::new();
    for channel_id in queue.take_dirty_channels() {
        live.insert(channel_id, queue.recoverable_triggers(channel_id));
    }
    ledger.commit(live);
}

// ── dispatch_pending ──────────────────────────────────────────────────────────

/// Drain every channel a public `&mut queue` mutator touched since the
/// last call and persist each one's current recoverable set. This is the
/// one sync rule the auto-resume ledger design relies on (see
/// `PLANS/AGENT_AUTO_RESUME_LEDGER.md` §"Core principle") — call it after
/// *every* public queue mutation so the durable mirror never drifts from
/// the in-memory queue.
fn sync_dirty(queue: &mut EventQueue, ledger: &mut Ledger) {
    for channel_id in queue.take_dirty_channels() {
        ledger.sync(channel_id, queue.recoverable_triggers(channel_id));
    }
}

/// Flush queued work to available agents.
fn dispatch_pending(
    pool: &mut AgentPool,
    queue: &mut EventQueue,
    ledger: &mut Ledger,
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
        let affinity_hit = pool.has_session_for(channel_id);
        let mut agent = match pool.try_claim(Some(channel_id)) {
            Some(a) => a,
            None => {
                let pending = queue.pending_channels();
                tracing::debug!(pending_channels = pending, "pool_exhausted");
                queue.complete_batch(
                    channel_id,
                    Some(batch),
                    queue::BatchDisposition::PreserveTimestamps,
                );
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

        // Goose-native non-cancelling steer seam: snapshot capability before
        // the agent moves into `run_prompt_task`, and install the per-turn
        // steer receiver on the read loop so the main loop's mode-gate fork
        // (see the `if accepted && queue.is_channel_in_flight(...)` block
        // in the relay event branch of the main `select!` loop) can drive
        // it via the matching sender stored in `TaskMeta.steer_tx`.
        // Install the steer channel for every prompt task — the supervisor
        // uses try-and-tolerate: it attempts the steer for any agent and
        // treats `-32601 method_not_found` as "fall back to cancel+merge".
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
                control_tx: Some(control_tx),
                steer_tx,
            },
        );
        dispatched_channels.push((channel_id, typing_scope));
    }
    sync_dirty(queue, ledger);
    tracing::debug!(
        dispatched = dispatched_channels.len(),
        queue_depth = queue.pending_channels(),
        "dispatch_pending"
    );
    dispatched_channels
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

/// Classify a completed prompt's batch (if any) into a [`BatchDisposition`]
/// and apply it via [`EventQueue::complete_batch`], posting a dead-letter
/// failure notice when retries are exhausted. Shared by the normal
/// completion path (`handle_prompt_result`) and the shutdown-drain paths
/// (R5-F2/R6-F4): both apply the identical disposition logic, but the
/// shutdown paths skip everything else here (respawn, heartbeat
/// bookkeeping, dispatch) — the caller decides what happens after.
fn classify_and_complete_batch(
    queue: &mut EventQueue,
    config: &Config,
    result: &mut PromptResult,
    removed_channels: &HashSet<Uuid>,
    rest_client: Option<&relay::RestClient>,
) -> Option<&'static str> {
    // Returns the hard-timeout fate suffix for the caller's death_message
    // construction (only meaningful when outcome is Timeout(Hard { .. }); None
    // for every other outcome).
    let mut hard_timeout_fate_suffix: Option<&'static str> = None;
    if let Some(batch) = result.batch.take() {
        let channel_id = batch.channel_id;
        // Don't requeue batches for channels the agent was removed from —
        // those events are stale and should be silently dropped.
        if removed_channels.contains(&channel_id) {
            tracing::debug!(
                channel_id = %channel_id,
                events = batch.events.len(),
                "dropping failed batch for removed channel"
            );
            queue.complete_batch(channel_id, Some(batch), BatchDisposition::Dropped);
            hard_timeout_fate_suffix = Some(" — batch dropped (channel removed)");
        } else if matches!(
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
            queue.complete_batch(channel_id, Some(batch), BatchDisposition::Cancelled(reason));
        } else if matches!(
            result.outcome,
            PromptOutcome::Timeout(TimeoutKind::Hard {
                recently_active: false
            })
        ) {
            // Hard-cap timeout with no recent activity is deterministic:
            // re-running the same task will reproduce the same death. Dead-letter
            // immediately without requeueing so the channel isn't subjected to
            // up to 10 × 1-hour retry cycles.
            tracing::error!(
                channel_id = %channel_id,
                events = batch.events.len(),
                "dead-lettering batch after hard-cap timeout (no recent activity) — discarding {} events",
                batch.events.len(),
            );
            let content = format!(
                "⚠️ I couldn't process the last request (the turn exceeded the maximum duration ({}s)). Please re-send if it's still needed.",
                config.max_turn_duration_secs
            );
            spawn_failure_notice(rest_client, &batch, content);
            queue.complete_batch(channel_id, Some(batch), BatchDisposition::Dropped);
            hard_timeout_fate_suffix = Some(" — dead-lettered (no recent activity)");
        } else if matches!(
            result.outcome,
            PromptOutcome::Timeout(TimeoutKind::Hard {
                recently_active: true
            })
        ) {
            // Hard-cap timeout with recent activity — requeue for retry; dead-letter
            // only once the retry budget is exhausted.
            tracing::warn!(
                channel_id = %channel_id,
                events = batch.events.len(),
                "hard-cap timeout with recent activity — requeueing for retry"
            );
            if let Some(dead) =
                queue.complete_batch(channel_id, Some(batch), BatchDisposition::Retry)
            {
                let content = format!(
                    "⚠️ I couldn't process the last request after multiple retries (the turn exceeded the maximum duration ({}s)). Please re-send if it's still needed.",
                    config.max_turn_duration_secs
                );
                spawn_failure_notice(rest_client, &dead, content);
                hard_timeout_fate_suffix = Some(" — dead-lettered (retry budget exhausted)");
            } else {
                hard_timeout_fate_suffix = Some(" — requeued for retry (recently active)");
            }
        } else {
            let outcome_reason = match &result.outcome {
                PromptOutcome::Timeout(TimeoutKind::Idle) => "the turn timed out".to_string(),
                PromptOutcome::Timeout(TimeoutKind::Hard { .. }) => {
                    "the turn exceeded the maximum duration".to_string()
                }
                PromptOutcome::AgentExited => "the agent process exited".to_string(),
                PromptOutcome::Error(e) => format!("{e}"),
                _ => "repeated failures".to_string(),
            };
            if let Some(dead) =
                queue.complete_batch(channel_id, Some(batch), BatchDisposition::Retry)
            {
                // Dead-lettered: retries exhausted and the events are gone.
                // Post a visible notice so the channel isn't left waiting on
                // a turn that will never happen.
                let content = format!(
                    "⚠️ I couldn't process the last request after multiple retries ({outcome_reason}). Please re-send if it's still needed."
                );
                spawn_failure_notice(rest_client, &dead, content);
            }
        }
    } else if let PromptSource::Channel(ch) = &result.source {
        queue.complete_batch(*ch, None, BatchDisposition::Success);
    }
    hard_timeout_fate_suffix
}

/// Best-effort drain of both `join_set` and `result_rx` during the shutdown
/// grace period. Tasks that finish normally send their `OwnedAgent` through
/// `result_rx` — reaped here to shut down child processes. If `grace`
/// expires with tasks still outstanding, the caller must abort the
/// remaining join_set tasks and fall back to `AcpClient::Drop` (best-effort,
/// not guaranteed).
///
/// Extracted from `run()`'s shutdown sequence so tests can drive the real
/// `select!` arms — including a task that panics mid-grace (R6-F4) and a
/// result that arrives mid-grace (R5-F2) — instead of calling
/// `queue.complete_batch()` directly and asserting the same disposition the
/// production code was supposed to apply.
async fn drain_shutdown_grace(
    pool: &mut AgentPool,
    queue: &mut EventQueue,
    ledger: &mut Ledger,
    config: &Config,
    removed_channels: &HashSet<Uuid>,
    rest_client: Option<&relay::RestClient>,
    grace: Duration,
) {
    let (rx_ref, js_ref, task_map_ref) = pool.rx_join_set_and_task_map();
    let shutdown_result = tokio::time::timeout(grace, async {
        loop {
            tokio::select! {
                result = js_ref.join_next_with_id() => {
                    match result {
                        Some(Err(join_error)) => {
                            tracing::warn!("task error during shutdown: {join_error}");
                            // R6-F4: a task that panics during the shutdown
                            // grace period produces no PromptResult, so its
                            // in-flight triggers would otherwise stay stale
                            // in the ledger mirror — Queue mode skipping
                            // retry accounting, Drop mode resurrecting a
                            // batch the normal panic policy would have
                            // dropped. Apply the same queue-only panic
                            // disposition `recover_panicked_agent` uses,
                            // minus respawn/dispatch (shutdown is tearing
                            // down, not recovering).
                            let task_id = join_error.id();
                            if let Some(meta) = task_map_ref.remove(&task_id) {
                                if let Some(ch) = meta.channel_id {
                                    let disposition = if removed_channels.contains(&ch) {
                                        BatchDisposition::Dropped
                                    } else {
                                        BatchDisposition::Retry
                                    };
                                    queue.complete_batch(ch, meta.recoverable_batch, disposition);
                                    sync_dirty(queue, ledger);
                                }
                            }
                        }
                        Some(Ok((_, ()))) => {}
                        None => break, // join_set empty
                    }
                }
                maybe_result = rx_ref.recv() => {
                    if let Some(mut pr) = maybe_result {
                        let idx = pr.agent.index;
                        classify_and_complete_batch(
                            queue,
                            config,
                            &mut pr,
                            removed_channels,
                            rest_client,
                        );
                        sync_dirty(queue, ledger);
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
    pool.task_map_mut()
        .retain(|_, meta| meta.agent_index != agent_index);
    debug_assert_eq!(before, pool.task_map().len() + 1);

    // The hard-timeout death_message (below) must describe the batch's
    // *actual* fate, not just the `recently_active` eligibility flag — a
    // recently-active batch that exhausts the retry budget in queue.requeue()
    // is dead-lettered same as an immediate one, and both differ from a
    // channel-removed drop or a heartbeat call with no batch at all. Each
    // branch below records what actually happened; only the hard-timeout
    // match arm in the death_message construction reads it.
    //
    // `classify_and_complete_batch` owns all disposition logic (P3-F1 —
    // one atomic completion writer) and returns the fate suffix for this
    // death_message.
    let hard_timeout_fate_suffix =
        classify_and_complete_batch(queue, config, &mut result, removed_channels, rest_client);

    if let PromptSource::Heartbeat = &result.source {
        *heartbeat_in_flight = false;
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
            );
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
    typing_channels: &mut HashMap<Uuid, ThreadTags>,
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

    // complete_batch owns disposition + mark_complete + dirty-tracking
    // atomically (same rationale as handle_prompt_result).
    if let Some(ch) = meta.channel_id {
        let disposition = if removed_channels.contains(&ch) {
            tracing::debug!(
                channel_id = %ch,
                "dropping panicked batch for removed channel"
            );
            BatchDisposition::Dropped
        } else {
            // Dead-letter on exhaustion is logged inside complete_batch(); a
            // panic path has no outcome to report, so no notice here.
            BatchDisposition::Retry
        };
        queue.complete_batch(ch, meta.recoverable_batch, disposition);
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
    typing_channels: &mut HashMap<Uuid, ThreadTags>,
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
                            "agent initialized — non-cancelling steer enabled (try-and-tolerate)"
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
        let session = client.session_new_full(&cwd, vec![], None).await?;
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
            context_message_limit: 12,
            max_turns_per_session: 0,
            presence_enabled: true,
            typing_enabled: true,
            memory_enabled: false,
            model: None,
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
            resume_on_restart: true,
            resume_ttl_secs: 0,
            state_dir: None,
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
            context_message_limit: 12,
            max_turns_per_session: 0,
            presence_enabled: true,
            typing_enabled: true,
            memory_enabled: false,
            model: None,
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
            resume_on_restart: true,
            resume_ttl_secs: 0,
            state_dir: None,
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
    async fn paul_probe_channel_released_after_ok_completion() {
        // Real main-loop cycle: push -> flush_next (marks in-flight) ->
        // Ok completion via handle_prompt_result -> channel must be
        // released so the next event can dispatch.
        //
        // Regression test for the merge-commit defect where `handle_prompt_result`
        // kept the inline requeue block (which never calls mark_complete) instead
        // of routing through `classify_and_complete_batch` (which does). Without
        // the fix, every channel wedges permanently after its first turn.
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let ev1 = EventBuilder::new(Kind::Custom(9), "first")
            .sign_with_keys(&keys)
            .unwrap();
        let ev2 = EventBuilder::new(Kind::Custom(9), "second")
            .sign_with_keys(&keys)
            .unwrap();

        let mut queue = EventQueue::new(config::DedupMode::Queue);
        queue.push(crate::queue::QueuedEvent::for_test(
            ch,
            ev1,
            std::time::Instant::now(),
            "test".into(),
        ));
        let _batch = queue.flush_next().expect("first flush");
        assert!(queue.is_channel_in_flight(ch));

        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![None]);
        let task_id = pool.join_set.spawn(async {}).id();
        pool.task_map_mut().insert(
            task_id,
            crate::pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(ch),
                turn_id: "t".to_string(),
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

        let result = PromptResult {
            agent,
            source: PromptSource::Channel(ch),
            turn_id: "t".to_string(),
            outcome: PromptOutcome::Ok(crate::acp::StopReason::EndTurn),
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
            None,
            None,
        );

        assert!(
            !queue.is_channel_in_flight(ch),
            "channel must be released after Ok completion"
        );
        queue.push(crate::queue::QueuedEvent::for_test(
            ch,
            ev2,
            std::time::Instant::now(),
            "test".into(),
        ));
        assert!(
            queue.flush_next().is_some(),
            "second event must dispatch after the turn completed"
        );
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
                events: vec![BatchEvent::for_test(
                    event,
                    "test".into(),
                    std::time::Instant::now(),
                )],
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
                    admission_seq: 0,
                    enqueued_at_unix: 0,
                    restart_recovery: false,
                    cap_exempt: false,
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
            events: vec![BatchEvent {
                event: EventBuilder::new(Kind::Custom(9), "test")
                    .sign_with_keys(&Keys::generate())
                    .unwrap(),
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
                admission_seq: 0,
                enqueued_at_unix: 0,
                restart_recovery: false,
                cap_exempt: false,
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
            events: vec![BatchEvent {
                event: EventBuilder::new(Kind::Custom(9), "final-attempt")
                    .sign_with_keys(&Keys::generate())
                    .unwrap(),
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
                admission_seq: 0,
                enqueued_at_unix: 0,
                restart_recovery: false,
                cap_exempt: false,
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
            events: vec![BatchEvent::for_test(
                original_event.clone(),
                "test".into(),
                std::time::Instant::now(),
            )],
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
        queue.push(QueuedEvent::for_test(
            channel_id,
            new_event.clone(),
            std::time::Instant::now(),
            "test".into(),
        ));
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

// ── Plan line 246 integration tests ──────────────────────────────────────────

#[cfg(test)]
mod boot_recovery_integration_tests {
    use super::*;
    use crate::config::DedupMode;
    use crate::ledger::{Ledger, LedgerRecord, StagedLedger};
    use crate::queue::{EventQueue, QueuedEvent};
    use crate::relay::RestClient;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::collections::{HashMap, HashSet};
    use std::time::{Duration, Instant};
    use tokio::io::AsyncWriteExt;
    use uuid::Uuid;

    fn make_channel_event(keys: &Keys, channel_id: Uuid, content: &str) -> nostr::Event {
        let h_tag = Tag::parse(["h", &channel_id.to_string()]).unwrap();
        EventBuilder::new(Kind::Custom(9), content)
            .tags([h_tag])
            .sign_with_keys(keys)
            .unwrap()
    }

    fn ledger_record(event: &nostr::Event, seq: u64, tag: &str) -> LedgerRecord {
        LedgerRecord {
            event_id: event.id.to_hex(),
            prompt_tag: tag.into(),
            admission_seq: seq,
            enqueued_at_unix: 1000 + seq,
            cap_exempt: false,
        }
    }

    fn ledger_record_exempt(event: &nostr::Event, seq: u64, tag: &str) -> LedgerRecord {
        LedgerRecord {
            event_id: event.id.to_hex(),
            prompt_tag: tag.into(),
            admission_seq: seq,
            enqueued_at_unix: 1000 + seq,
            cap_exempt: true,
        }
    }

    fn rest_client(base_url: &str) -> RestClient {
        RestClient {
            http: reqwest::Client::new(),
            base_url: base_url.into(),
            keys: Keys::generate(),
            auth_tag_json: None,
        }
    }

    /// Same shape as `error_outcome_emission_tests::test_config` /
    /// `build_mcp_servers_tests::test_config` — duplicated here because
    /// those helpers are private to their own test modules. Only used by
    /// the shutdown-drain tests, which need a real `Config` to pass to
    /// `drain_shutdown_grace`/`classify_and_complete_batch`.
    fn test_config() -> Config {
        Config {
            keys: Keys::generate(),
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
            resume_on_restart: true,
            resume_ttl_secs: 0,
            state_dir: None,
        }
    }

    /// Minimal `PromptContext` for driving `dispatch_pending` directly —
    /// same shape as `pool::tests::make_prompt_context_impl`, duplicated
    /// here because that helper is private to `pool`'s test module.
    fn test_prompt_context() -> Arc<PromptContext> {
        test_prompt_context_with_dedup_mode(DedupMode::Queue)
    }

    /// Variant of [`test_prompt_context`] with an explicit `dedup_mode` —
    /// `dispatch_pending` reads `ctx.dedup_mode` (not the `EventQueue`'s
    /// own mode) to decide whether to clone a recoverable batch into
    /// `TaskMeta`, so panic/shutdown-drain tests that need to exercise
    /// `DedupMode::Drop`'s "nothing recovered" behavior must set it here.
    fn test_prompt_context_with_dedup_mode(dedup_mode: DedupMode) -> Arc<PromptContext> {
        let agent_keys = Keys::generate();
        Arc::new(PromptContext {
            mcp_servers: vec![],
            initial_message: None,
            idle_timeout: Duration::from_secs(60),
            max_turn_duration: Duration::from_secs(120),
            turn_liveness_interval: Duration::ZERO,
            dedup_mode,
            system_prompt: None,
            team_instructions: None,
            heartbeat_prompt: None,
            base_prompt: None,
            cwd: ".".to_string(),
            rest_client: rest_client("http://127.0.0.1:0"),
            channel_info: HashMap::new(),
            context_message_limit: 0,
            max_turns_per_session: 0,
            permission_mode: crate::config::PermissionMode::Default,
            agent_keys: agent_keys.clone(),
            agent_owner_pubkey: None,
            memory_enabled: false,
            harness_name: "goose".to_string(),
            relay_url: "ws://localhost:3000".to_string(),
        })
    }

    /// Spawn a real but inert agent subprocess (`cat`) so `dispatch_pending`
    /// has a genuine `OwnedAgent` to claim — mirrors `dummy_agent` in the
    /// top-level test module (private there, duplicated here for the same
    /// reason as `test_prompt_context` above).
    async fn dummy_agent(index: usize) -> pool::OwnedAgent {
        pool::OwnedAgent {
            index,
            acp: acp::AcpClient::spawn("cat", &[], &[], false)
                .await
                .expect("spawn cat as inert agent"),
            state: Default::default(),
            model_capabilities: None,
            desired_model: None,
            model_overridden: false,
            agent_name: "unknown".into(),
            goose_system_prompt_supported: None,
            protocol_version: 1,
        }
    }

    async fn mock_rest_server(events: Vec<nostr::Event>) -> (tokio::task::JoinHandle<()>, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");
        let handle = tokio::spawn(async move {
            let events_json = serde_json::to_string(&events).unwrap();
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = vec![0u8; 8192];
                let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    events_json.len(),
                    events_json
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        (handle, base_url)
    }

    /// Like `mock_rest_server`, but serves raw JSON values instead of
    /// `nostr::Event`s — lets tests inject malformed/forged/tampered
    /// responses (e.g. a corrupted `sig`) that a valid `nostr::Event` could
    /// never represent.
    async fn mock_rest_server_values(
        values: Vec<serde_json::Value>,
    ) -> (tokio::task::JoinHandle<()>, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");
        let handle = tokio::spawn(async move {
            let events_json = serde_json::to_string(&values).unwrap();
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = vec![0u8; 8192];
                let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    events_json.len(),
                    events_json
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        (handle, base_url)
    }

    /// Like `mock_rest_server`, but sleeps for `delay` (a tokio timer, so it
    /// respects `start_paused` + `tokio::time::advance`) after reading the
    /// request and before writing the response — lets tests simulate fetch
    /// latency that must be advanced through WHILE `boot_recover`'s fetch
    /// loop is actually awaiting the response, not before it starts.
    async fn mock_rest_server_delayed(
        events: Vec<nostr::Event>,
        delay: Duration,
    ) -> (tokio::task::JoinHandle<()>, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");
        let handle = tokio::spawn(async move {
            let events_json = serde_json::to_string(&events).unwrap();
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = vec![0u8; 8192];
                let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
                tokio::time::sleep(delay).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    events_json.len(),
                    events_json
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        (handle, base_url)
    }

    async fn mock_rest_server_failing() -> (tokio::task::JoinHandle<()>, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let _ = stream.shutdown().await;
            }
        });
        (handle, base_url)
    }

    // ── Scenario 1: push → kill → boot scan → resume ─────────────────────

    #[tokio::test]
    async fn test_boot_recover_resumes_queued_with_original_tag_and_flag() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "hello");
        let record_a = ledger_record(&event_a, 1, "mention");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![record_a])]),
        };

        let (server, base_url) = mock_rest_server(vec![event_a.clone()]).await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let mut ledger = Ledger::disabled();
        let live_channels: HashSet<Uuid> = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].event_id, event_a.id.to_hex());
        assert_eq!(triggers[0].prompt_tag, "mention");
        assert_eq!(triggers[0].admission_seq, 1);

        let batch = queue.flush_next().unwrap();
        assert!(batch.events[0].restart_recovery);

        server.abort();
    }

    // ── Scenario 2: push → complete → scan → empty ───────────────────────

    #[tokio::test]
    async fn test_boot_recover_empty_ledger_produces_empty_queue() {
        let staged = StagedLedger::default();
        let rest = rest_client("http://127.0.0.1:1");
        let mut queue = EventQueue::new(DedupMode::Queue);
        let mut ledger = Ledger::disabled();
        let live_channels = HashSet::new();
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        assert!(queue.flush_next().is_none());
        assert!(suppression.is_empty());
    }

    // ── Scenario 3: suppression both arrival orders ──────────────────────

    #[tokio::test]
    async fn test_suppression_set_prevents_duplicate_live_push() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "msg");
        let record_a = ledger_record(&event_a, 1, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![record_a])]),
        };

        let (server, base_url) = mock_rest_server(vec![event_a.clone()]).await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let mut ledger = Ledger::disabled();
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;
        assert!(suppression.contains(&event_a.id.to_hex()));

        let is_suppressed = suppression.remove(&event_a.id.to_hex());
        assert!(is_suppressed);

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 1, "only the boot-imported copy exists");

        server.abort();
    }

    #[tokio::test]
    async fn test_suppression_live_arrives_first_then_boot_finds_same() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "msg");

        let mut queue = EventQueue::new(DedupMode::Queue);
        queue.push(QueuedEvent::new(
            ch,
            event_a.clone(),
            Instant::now(),
            "test".into(),
        ));

        let record_a = ledger_record(&event_a, 1, "test");
        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![record_a])]),
        };

        let (server, base_url) = mock_rest_server(vec![event_a.clone()]).await;
        let rest = rest_client(&base_url);

        let mut ledger = Ledger::disabled();
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(
            triggers.len(),
            2,
            "both the live push and the boot import are in the queue"
        );

        assert!(suppression.contains(&event_a.id.to_hex()));

        server.abort();
    }

    // Mode matrix: the two arrival-order tests above pin `DedupMode::Queue`.
    // `boot_recover` populates `recovered_suppression` identically
    // regardless of dedup mode (the mode only affects `EventQueue::push`,
    // which boot_recover never calls — it uses `import_recovered`), so the
    // discriminating check is the same suppression-set assertion under
    // `DedupMode::Drop`.
    #[tokio::test]
    async fn test_suppression_set_prevents_duplicate_live_push_drop_mode() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "msg");
        let record_a = ledger_record(&event_a, 1, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![record_a])]),
        };

        let (server, base_url) = mock_rest_server(vec![event_a.clone()]).await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Drop);
        let mut ledger = Ledger::disabled();
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;
        assert!(suppression.contains(&event_a.id.to_hex()));

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 1, "only the boot-imported copy exists");

        server.abort();
    }

    // "Replay-only for non-ledger events" (plan:192): an event that was
    // never staged in the ledger must never appear in the suppression set,
    // regardless of dedup mode — the live-admission seam's suppression
    // check is keyed strictly on ids `boot_recover` actually imported.
    #[tokio::test]
    async fn test_suppression_does_not_affect_non_ledger_event() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_ledger = make_channel_event(&keys, ch, "recovered");
        let event_fresh = make_channel_event(&keys, ch, "never-staged");
        let record = ledger_record(&event_ledger, 1, "test");

        let (server, base_url) = mock_rest_server(vec![event_ledger.clone()]).await;
        let rest = rest_client(&base_url);

        for mode in [DedupMode::Queue, DedupMode::Drop] {
            let staged = StagedLedger {
                channels: HashMap::from([(ch, vec![record.clone()])]),
            };
            let mut queue = EventQueue::new(mode);
            let mut ledger = Ledger::disabled();
            let live_channels = HashSet::from([ch]);
            let mut suppression = HashSet::new();

            boot_recover(
                &mut queue,
                &mut ledger,
                staged,
                &live_channels,
                &rest,
                &mut suppression,
            )
            .await;

            assert!(
                !suppression.contains(&event_fresh.id.to_hex()),
                "a never-staged event's id must never land in the suppression set ({mode:?})"
            );
        }

        server.abort();
    }

    // "with an event timestamped inside the skew window" (plan:192): the
    // suppression set is a pure id-based match, but the timing scenario it
    // exists to guard is a fast restart where the relay's reconnect `since`
    // subtracts `SINCE_SKEW_SECS` (relay.rs) and could replay an event
    // admitted just before the crash. Pin that the suppression check does
    // not depend on how recently the record was enqueued — a record
    // enqueued at "now" (the skew-window case) suppresses identically to
    // one enqueued long ago.
    #[tokio::test]
    async fn test_suppression_covers_event_inside_skew_window() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "just-before-crash");
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let record_a = LedgerRecord {
            event_id: event_a.id.to_hex(),
            prompt_tag: "test".into(),
            admission_seq: 1,
            // Enqueued inside the SINCE_SKEW_SECS window relative to "now" —
            // the exact case where a fast restart's reconnect replay could
            // otherwise double-admit this event over WS.
            enqueued_at_unix: now_unix.saturating_sub(2),
            cap_exempt: false,
        };

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![record_a])]),
        };

        let (server, base_url) = mock_rest_server(vec![event_a.clone()]).await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let mut ledger = Ledger::disabled();
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        assert!(
            suppression.contains(&event_a.id.to_hex()),
            "a recently-enqueued (skew-window) record must still be suppressed \
             against a WS replay of the same event"
        );

        server.abort();
    }

    // ── Scenario 4: membership gate drops unsubscribed channels ──────────

    #[tokio::test]
    async fn test_boot_recover_membership_gate_drops_absent_channel() {
        let keys = Keys::generate();
        let ch_live = Uuid::new_v4();
        let ch_gone = Uuid::new_v4();
        let ev_live = make_channel_event(&keys, ch_live, "live");
        let ev_gone = make_channel_event(&keys, ch_gone, "gone");
        let rec_live = ledger_record(&ev_live, 1, "test");
        let rec_gone = ledger_record(&ev_gone, 2, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch_live, vec![rec_live]), (ch_gone, vec![rec_gone])]),
        };

        let (server, base_url) = mock_rest_server(vec![ev_live.clone()]).await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let mut ledger = Ledger::disabled();
        let live_channels = HashSet::from([ch_live]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        assert_eq!(queue.recoverable_triggers(ch_live).len(), 1);
        assert!(queue.recoverable_triggers(ch_gone).is_empty());

        server.abort();
    }

    #[tokio::test]
    async fn test_boot_recover_all_channels_dropped_commits_empty_ledger() {
        let keys = Keys::generate();
        let ch_gone = Uuid::new_v4();
        let ev_gone = make_channel_event(&keys, ch_gone, "stale");

        let dir = tempfile::tempdir().unwrap();

        // Persist stale state to disk through the real API so the file
        // exists before boot_recover runs.
        {
            let (mut ledger0, _) =
                Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
            let trigger = queue::RecoverableTrigger {
                event_id: ev_gone.id.to_hex(),
                prompt_tag: "test".into(),
                admission_seq: 1,
                enqueued_at_unix: 1001,
                cap_exempt: false,
            };
            ledger0.sync(ch_gone, vec![trigger]);
        }

        // Reload — staged must come from disk, not memory.
        let (mut ledger, staged) =
            Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        assert!(
            !staged.channels.is_empty(),
            "setup guard: stale record must exist on disk before recovery"
        );

        let rest = rest_client("http://127.0.0.1:1");
        let mut queue = EventQueue::new(DedupMode::Queue);
        let live_channels: HashSet<Uuid> = HashSet::new();
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        let (_ledger2, staged2) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        assert!(
            staged2.channels.is_empty(),
            "all-dropped boot must commit an empty ledger — stale records must not survive on disk"
        );
    }

    // ── Scenario 5: REST-fail → WS-deliver → no duplicate ───────────────

    #[tokio::test]
    async fn test_rest_fail_then_live_resolve_no_duplicate() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "msg");
        let record_a = ledger_record(&event_a, 1, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![record_a.clone()])]),
        };

        let (server, base_url) = mock_rest_server_failing().await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        assert!(
            queue.recoverable_triggers(ch).is_empty(),
            "REST failed — nothing imported"
        );
        assert!(suppression.is_empty());

        let unresolved = ledger.find_unresolved(ch, &event_a.id.to_hex());
        assert!(unresolved.is_some(), "unfetched id retained as unresolved");

        let recovered = QueuedEvent::from_recovered(
            ch,
            event_a.clone(),
            record_a.prompt_tag.clone(),
            record_a.admission_seq,
            record_a.enqueued_at_unix,
            record_a.cap_exempt,
        );
        queue.admit_recovered(ch, recovered);
        ledger.resolve_unresolved(ch, &event_a.id.to_hex());

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].event_id, event_a.id.to_hex());

        assert!(ledger.find_unresolved(ch, &event_a.id.to_hex()).is_none());

        server.abort();
    }

    // ── Scenario 6: REST-fail → removal → re-add → no resurrection ──────

    #[tokio::test]
    async fn test_rest_fail_channel_removed_then_readded_no_resurrection() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "msg");
        let record_a = ledger_record(&event_a, 1, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![record_a])]),
        };

        let (server, base_url) = mock_rest_server_failing().await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;
        assert!(ledger.find_unresolved(ch, &event_a.id.to_hex()).is_some());

        ledger.invalidate_channel(ch);
        assert!(ledger.find_unresolved(ch, &event_a.id.to_hex()).is_none());

        let new_event = make_channel_event(&keys, ch, "new");
        queue.push(QueuedEvent::new(
            ch,
            new_event.clone(),
            Instant::now(),
            "test".into(),
        ));

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 1);
        assert_eq!(
            triggers[0].event_id,
            new_event.id.to_hex(),
            "only the new event, not the old one"
        );

        server.abort();
    }

    // ── Scenario 7: failed fetch → retained → resolved on next restart ──

    #[tokio::test]
    async fn test_unresolved_retained_across_restart_then_resolved() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "msg");
        let record_a = ledger_record(&event_a, 1, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![record_a])]),
        };

        let (server, base_url) = mock_rest_server_failing().await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;
        assert!(ledger.find_unresolved(ch, &event_a.id.to_hex()).is_some());
        server.abort();

        let (server2, base_url2) = mock_rest_server(vec![event_a.clone()]).await;
        let rest2 = rest_client(&base_url2);

        let mut queue2 = EventQueue::new(DedupMode::Queue);
        let unresolved_record = ledger.find_unresolved(ch, &event_a.id.to_hex()).unwrap();
        let staged2 = StagedLedger {
            channels: HashMap::from([(ch, vec![unresolved_record])]),
        };
        let mut suppression2 = HashSet::new();

        boot_recover(
            &mut queue2,
            &mut ledger,
            staged2,
            &live_channels,
            &rest2,
            &mut suppression2,
        )
        .await;

        let triggers = queue2.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].event_id, event_a.id.to_hex());

        server2.abort();
    }

    // ── Scenario 8: successful shutdown-grace result → no recovery ────────
    //
    // All four scenarios below (8, 9, 10a, 10b) drive the real
    // `drain_shutdown_grace` seam (lib.rs) — the exact `select!`/timeout
    // wrapper `run()` awaits during Ctrl+C shutdown — instead of calling
    // `queue.complete_batch()` directly and asserting the disposition the
    // production code was "supposed to" apply. A regression that stops
    // routing a shutdown-grace result/panic through `classify_and_complete_batch`
    // (or drops the panic-handling arm entirely) now fails these tests.

    /// Dispatch one real in-flight task via `dispatch_pending` so the pool's
    /// `task_map`/`join_set` are in the exact state `drain_shutdown_grace`
    /// expects at shutdown — mirrors the production call sequence
    /// (`dispatch_pending` in the main loop, then `drain_shutdown_grace` at
    /// Ctrl+C) instead of hand-rolling a `TaskMeta` insert.
    async fn dispatch_one_in_flight(
        queue: &mut EventQueue,
        ledger: &mut Ledger,
        ch: Uuid,
        event: &nostr::Event,
    ) -> AgentPool {
        queue.push(QueuedEvent::new(
            ch,
            event.clone(),
            Instant::now(),
            "test".into(),
        ));
        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![Some(agent)]);
        let ctx = test_prompt_context();
        let dispatched = dispatch_pending(&mut pool, queue, ledger, &ctx);
        assert_eq!(dispatched.len(), 1, "setup: exactly one batch dispatched");
        pool
    }

    #[tokio::test]
    async fn test_successful_shutdown_grace_result_clears_triggers() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "msg");

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let mut pool = dispatch_one_in_flight(&mut queue, &mut ledger, ch, &event_a).await;
        assert!(
            !queue.recoverable_triggers(ch).is_empty(),
            "in-flight triggers present before shutdown"
        );

        // The in-flight task is `cat` — it never sends a PromptResult on its
        // own, so drive success by pushing one through the same
        // `result_tx` the pool hands the real spawned task, then let
        // `drain_shutdown_grace` reap it.
        let agent = dummy_agent(1).await;
        let result_tx = pool.result_tx();
        result_tx
            .send(PromptResult {
                agent,
                source: PromptSource::Channel(ch),
                turn_id: "shutdown-test".into(),
                outcome: PromptOutcome::Ok(crate::acp::StopReason::EndTurn),
                batch: None, // Success path: handle_prompt_result also passes None here.
            })
            .unwrap();

        let config = test_config();
        let removed_channels = HashSet::new();
        drain_shutdown_grace(
            &mut pool,
            &mut queue,
            &mut ledger,
            &config,
            &removed_channels,
            None,
            Duration::from_secs(5),
        )
        .await;

        assert!(
            queue.recoverable_triggers(ch).is_empty(),
            "no triggers after successful completion drains through the real grace loop"
        );
    }

    // ── Scenario 9: failed shutdown-grace result → triggers persisted ────

    #[tokio::test]
    async fn test_failed_shutdown_grace_result_persists_triggers() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "msg");

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let mut pool = dispatch_one_in_flight(&mut queue, &mut ledger, ch, &event_a).await;

        // Recover the batch dispatch_pending cloned into TaskMeta so the
        // failure result carries the same recoverable batch the real
        // read-loop would attach on an error outcome.
        let recoverable_batch = pool
            .task_map()
            .values()
            .find(|m| m.channel_id == Some(ch))
            .and_then(|m| m.recoverable_batch.clone())
            .expect("dispatch_pending must have cloned a recoverable batch in Queue mode");

        let agent = dummy_agent(1).await;
        let result_tx = pool.result_tx();
        result_tx
            .send(PromptResult {
                agent,
                source: PromptSource::Channel(ch),
                turn_id: "shutdown-test".into(),
                outcome: PromptOutcome::AgentExited,
                batch: Some(recoverable_batch),
            })
            .unwrap();

        let config = test_config();
        let removed_channels = HashSet::new();
        drain_shutdown_grace(
            &mut pool,
            &mut queue,
            &mut ledger,
            &config,
            &removed_channels,
            None,
            Duration::from_secs(5),
        )
        .await;

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(
            triggers.len(),
            1,
            "triggers requeued after a failure result arrives mid-grace"
        );
        assert_eq!(triggers[0].event_id, event_a.id.to_hex());
    }

    // ── Scenario 10: panic during grace in both dedup modes (R6-F4) ──────

    /// Drive a real join_set task panic through `drain_shutdown_grace`,
    /// asserting the post-panic recoverable_triggers state for `dedup_mode`.
    /// Returns `(queue, channel_id)` since the test drives channel state
    /// through `recoverable_triggers`, which needs the id back.
    async fn panic_during_grace(dedup_mode: DedupMode) -> (EventQueue, Uuid) {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "msg");

        let mut queue = EventQueue::new(dedup_mode);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        queue.push(QueuedEvent::new(
            ch,
            event_a.clone(),
            Instant::now(),
            "test".into(),
        ));

        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![Some(agent)]);
        let ctx = test_prompt_context_with_dedup_mode(dedup_mode);
        let dispatched = dispatch_pending(&mut pool, &mut queue, &mut ledger, &ctx);
        assert_eq!(dispatched.len(), 1, "setup: exactly one batch dispatched");

        // Real join_set task panics mid-grace: abort the dispatched task
        // (the actual spawned `cat`-reading prompt task) instead of
        // spawning a synthetic panicking future — this is the exact
        // in-flight task drain_shutdown_grace's join_next_with_id() arm
        // must classify via task_map, not a hand-inserted TaskMeta.
        // `abort_all` is used (JoinSet exposes no by-id abort) — safe here
        // because exactly one task is ever in flight in this harness.
        assert_eq!(
            pool.task_map().len(),
            1,
            "exactly one in-flight task to abort"
        );
        pool.join_set.abort_all();

        let config = test_config();
        let removed_channels = HashSet::new();
        drain_shutdown_grace(
            &mut pool,
            &mut queue,
            &mut ledger,
            &config,
            &removed_channels,
            None,
            Duration::from_secs(5),
        )
        .await;

        (queue, ch)
    }

    #[tokio::test]
    async fn test_panic_during_grace_queue_mode_retries() {
        let (queue, ch) = panic_during_grace(DedupMode::Queue).await;
        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 1, "Queue mode: panic retries");
    }

    #[tokio::test]
    async fn test_panic_during_grace_drop_mode_recovers_nothing() {
        let (queue, ch) = panic_during_grace(DedupMode::Drop).await;
        assert!(
            queue.recoverable_triggers(ch).is_empty(),
            "Drop mode: nothing recovered after a mid-grace panic"
        );
    }

    // ── Scenario 11: large ledger + hanging bridge → budget ──────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_large_ledger_hanging_bridge_respects_deadline() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let mut records = Vec::new();
        let mut events = Vec::new();
        for i in 0..150 {
            let ev = make_channel_event(&keys, ch, &format!("msg-{i}"));
            records.push(ledger_record(&ev, i as u64, "test"));
            events.push(ev);
        }

        let staged = StagedLedger {
            channels: HashMap::from([(ch, records)]),
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = vec![0u8; 8192];
                let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
                tokio::time::sleep(Duration::from_secs(120)).await;
                let _ = stream.shutdown().await;
            }
        });

        let rest = rest_client(&base_url);
        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        let start = tokio::time::Instant::now();
        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_secs(BOOT_RECOVERY_FETCH_DEADLINE.as_secs() + 15),
            "boot_recover must respect its deadline, took {elapsed:?}"
        );

        let unresolved = ledger.unresolved_channels();
        assert!(
            !unresolved.is_empty(),
            "hanging bridge leaves triggers unresolved"
        );

        // The plan's acceptance criterion is "boot reaches dispatch within
        // the global budget", not merely "returns". `boot_recover` having
        // returned already proves that (the fetch loop cannot outlive
        // `BOOT_RECOVERY_FETCH_DEADLINE` — asserted above); this additionally
        // confirms boot reached the barrier-registration step rather than
        // hanging or panicking partway through the fetch loop.
        assert!(
            queue.next_unresolved_barrier_deadline().is_some(),
            "boot must reach the barrier-registration step, not hang in the fetch loop"
        );

        server.abort();
    }

    // ── Scenario 12: chunk reconciliation ────────────────────────────────

    #[tokio::test]
    async fn test_chunk_reconciliation_valid_events_imported() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let ev = make_channel_event(&keys, ch, "valid");
        let rec = ledger_record(&ev, 1, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![rec])]),
        };

        let (server, base_url) = mock_rest_server(vec![ev.clone()]).await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let mut ledger = Ledger::disabled();
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        assert_eq!(queue.recoverable_triggers(ch).len(), 1);
        server.abort();
    }

    #[tokio::test]
    async fn test_chunk_reconciliation_wrong_channel_event_stays_unresolved() {
        let keys = Keys::generate();
        let ch_expected = Uuid::new_v4();
        let ch_wrong = Uuid::new_v4();
        let ev = make_channel_event(&keys, ch_wrong, "wrong-channel");
        let ev_id_hex = ev.id.to_hex();
        let rec = LedgerRecord {
            event_id: ev_id_hex.clone(),
            prompt_tag: "test".into(),
            admission_seq: 1,
            enqueued_at_unix: 1001,
            cap_exempt: false,
        };

        let staged = StagedLedger {
            channels: HashMap::from([(ch_expected, vec![rec])]),
        };

        let (server, base_url) = mock_rest_server(vec![ev]).await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let live_channels = HashSet::from([ch_expected]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        assert!(queue.recoverable_triggers(ch_expected).is_empty());
        assert!(ledger.find_unresolved(ch_expected, &ev_id_hex).is_some());

        server.abort();
    }

    #[tokio::test]
    async fn test_chunk_reconciliation_duplicate_events_import_once() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let ev = make_channel_event(&keys, ch, "dup");
        let rec = ledger_record(&ev, 1, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![rec])]),
        };

        let (server, base_url) = mock_rest_server(vec![ev.clone(), ev.clone()]).await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let mut ledger = Ledger::disabled();
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        assert_eq!(
            queue.recoverable_triggers(ch).len(),
            1,
            "duplicate response imports exactly once"
        );
        server.abort();
    }

    #[tokio::test]
    async fn test_chunk_reconciliation_unrequested_event_id_ignored() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let ev_requested = make_channel_event(&keys, ch, "requested");
        let ev_extra = make_channel_event(&keys, ch, "extra-unrequested");
        let rec = ledger_record(&ev_requested, 1, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![rec])]),
        };

        let (server, base_url) = mock_rest_server(vec![ev_requested.clone(), ev_extra]).await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let mut ledger = Ledger::disabled();
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].event_id, ev_requested.id.to_hex());
        server.abort();
    }

    #[tokio::test]
    async fn test_chunk_reconciliation_invalid_signature_stays_unresolved() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let ev_good = make_channel_event(&keys, ch, "valid");
        let ev_forged = make_channel_event(&keys, ch, "forged");
        let rec_good = ledger_record(&ev_good, 1, "test");
        let rec_forged = ledger_record(&ev_forged, 2, "test");

        // Tamper the forged event's signature after signing — the id is
        // still requested (matches the ledger record), but verification
        // must fail, so the record must stay unresolved-retained rather
        // than importing untrusted content (R6-F5).
        let mut forged_json = serde_json::to_value(&ev_forged).unwrap();
        let tampered_sig = "0".repeat(128);
        forged_json["sig"] = serde_json::Value::String(tampered_sig);

        let good_json = serde_json::to_value(&ev_good).unwrap();
        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![rec_good, rec_forged])]),
        };

        let (server, base_url) = mock_rest_server_values(vec![good_json, forged_json]).await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 1, "only the validly-signed event imports");
        assert_eq!(triggers[0].event_id, ev_good.id.to_hex());
        assert!(
            ledger.find_unresolved(ch, &ev_forged.id.to_hex()).is_some(),
            "invalid-signature event must stay unresolved-retained, never imported"
        );

        server.abort();
    }

    #[tokio::test]
    async fn test_chunk_reconciliation_omitted_id_stays_unresolved() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let ev_returned = make_channel_event(&keys, ch, "returned");
        let ev_omitted = make_channel_event(&keys, ch, "omitted-by-bridge");
        let rec_returned = ledger_record(&ev_returned, 1, "test");
        let rec_omitted = ledger_record(&ev_omitted, 2, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![rec_returned, rec_omitted])]),
        };

        // Bridge returns only one of the two requested ids — a partial
        // chunk response, not a failure.
        let (server, base_url) = mock_rest_server(vec![ev_returned.clone()]).await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].event_id, ev_returned.id.to_hex());
        assert!(
            ledger
                .find_unresolved(ch, &ev_omitted.id.to_hex())
                .is_some(),
            "requested-but-omitted id must stay unresolved-retained, never dropped"
        );

        server.abort();
    }

    // ── Scenario 13: steer short-circuit (R6-F2) ─────────────────────────

    #[tokio::test]
    async fn test_unresolved_resolution_skips_steer_path() {
        // Drives the real admission decision (`admit_live_event`, the exact
        // function the main loop's relay-event arm calls) instead of
        // hand-rolling the ledger/queue calls and asserting a hardcoded
        // `skip_steer = true`. A regression that stops routing resolving
        // events through the unresolved-lookup branch (e.g. skips
        // `find_unresolved` or returns `skip_steer = false`) now fails this
        // test.
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "msg");
        let record_a = ledger_record(&event_a, 1, "test");

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let mut suppression: HashSet<String> = HashSet::new();

        ledger.add_unresolved(ch, record_a.clone());

        // The live relay redelivers event_a — same path the main loop takes
        // when a resolving event arrives for an unresolved barrier record.
        let (accepted, skip_steer) = admit_live_event(
            &mut queue,
            &mut ledger,
            &mut suppression,
            ch,
            event_a.clone(),
            "test".into(),
        );

        assert!(accepted, "resolving event must be admitted into the queue");
        assert!(
            skip_steer,
            "resolved recovered event must skip native steer"
        );

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].event_id, event_a.id.to_hex());
        assert!(
            ledger.find_unresolved(ch, &event_a.id.to_hex()).is_none(),
            "unresolved record must be consumed on resolution"
        );
    }

    #[tokio::test]
    async fn test_live_event_with_no_unresolved_record_takes_steer_path() {
        // Companion to the scenario above: an ordinary live event with no
        // matching unresolved ledger record and no suppression entry must
        // go through the normal `queue.push` admission and NOT skip steer.
        // Without this case, a mutation that makes `admit_live_event`
        // always return `skip_steer = true` would slip through undetected.
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "unrelated");

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let mut suppression: HashSet<String> = HashSet::new();

        let (accepted, skip_steer) = admit_live_event(
            &mut queue,
            &mut ledger,
            &mut suppression,
            ch,
            event_a.clone(),
            "test".into(),
        );

        assert!(accepted, "ordinary live event must be admitted");
        assert!(
            !skip_steer,
            "an ordinary live event with no unresolved record must take the native steer path"
        );
    }

    #[tokio::test]
    async fn test_recovered_suppressed_event_skips_steer_and_is_not_readmitted() {
        // Third branch: a duplicate live delivery for an event already
        // imported by boot recovery. Must be suppressed (not pushed again)
        // and must still skip steer.
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "already-imported");

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let mut suppression: HashSet<String> = HashSet::from([event_a.id.to_hex()]);

        let (accepted, skip_steer) = admit_live_event(
            &mut queue,
            &mut ledger,
            &mut suppression,
            ch,
            event_a.clone(),
            "test".into(),
        );

        assert!(
            !accepted,
            "suppressed duplicate must not be pushed into the queue again"
        );
        assert!(skip_steer, "suppressed duplicate must skip native steer");
        assert!(
            !suppression.contains(&event_a.id.to_hex()),
            "suppression entry must be consumed on the matching live delivery"
        );
    }

    // ── Scenario 14: quiet-harness barrier expiry (BINDING) ──────────────

    #[tokio::test]
    async fn test_quiet_harness_barrier_expiry_dispatches_with_no_external_event() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();

        let event_a = make_channel_event(&keys, ch, "unresolved-A");
        let event_b = make_channel_event(&keys, ch, "fetched-B");

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);

        let recovered_b =
            QueuedEvent::from_recovered(ch, event_b.clone(), "test".into(), 2, 1002, false);
        queue.import_recovered(ch, vec![recovered_b]);

        let record_a = ledger_record(&event_a, 1, "test");
        ledger.add_unresolved(ch, record_a);

        let barrier_deadline = Instant::now() + Duration::from_millis(1);
        let mut unresolved_seqs = std::collections::BTreeSet::new();
        unresolved_seqs.insert(1);
        queue.set_unresolved_barrier(ch, unresolved_seqs, barrier_deadline);

        queue.take_dirty_channels();

        assert_eq!(
            queue.recoverable_triggers(ch).len(),
            1,
            "B imported, A unresolved"
        );
        assert!(ledger.find_unresolved(ch, &event_a.id.to_hex()).is_some());

        let deadline = queue.next_unresolved_barrier_deadline();
        assert!(deadline.is_some(), "barrier must be armed");

        tokio::time::sleep(Duration::from_millis(5)).await;

        let expired = queue.expire_due_unresolved_barriers(Instant::now());
        assert!(!expired.is_empty(), "barrier must have expired");
        assert!(expired.contains(&ch));

        sync_dirty(&mut queue, &mut ledger);

        assert!(
            queue.has_flushable_work(),
            "after barrier expiry, B must be flushable"
        );

        let batch = queue.flush_next().unwrap();
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].event.id, event_b.id);
        assert!(batch.events[0].restart_recovery);
    }

    // Not `start_paused`: `expire_due_unresolved_barriers` takes
    // `std::time::Instant::now()` (real wall clock) in the production
    // select! arm at :2270. Under a paused virtual clock that call can
    // never observe the advance — driving it with a synthetic
    // `past_deadline` (as the old version of this test did) hides that
    // divergence entirely, since the synthetic value passes regardless of
    // whether real time elapsed. Use a short real deadline instead and run
    // the identical production expression on the real clock.
    #[tokio::test]
    async fn test_quiet_harness_select_timer_arm_wakes_loop() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();

        let _event_a = make_channel_event(&keys, ch, "unresolved-A");
        let event_b = make_channel_event(&keys, ch, "fetched-B");

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);

        let recovered_b =
            QueuedEvent::from_recovered(ch, event_b.clone(), "test".into(), 2, 1002, false);
        queue.import_recovered(ch, vec![recovered_b]);

        let barrier_deadline = std::time::Instant::now() + Duration::from_millis(50);
        let mut unresolved_seqs = std::collections::BTreeSet::new();
        unresolved_seqs.insert(1);
        queue.set_unresolved_barrier(ch, unresolved_seqs, barrier_deadline);

        queue.take_dirty_channels();

        assert!(!queue.has_flushable_work());

        // Same construct as the production select! timer arm at :2262-2266:
        // convert the barrier's std deadline to a tokio deadline and sleep
        // on it. The real clock (not virtual) must cross 50ms for this to
        // resolve.
        let barrier_std_deadline = queue.next_unresolved_barrier_deadline().unwrap();
        let tokio_deadline = tokio::time::Instant::from_std(barrier_std_deadline);
        tokio::time::sleep_until(tokio_deadline).await;

        // Same production expression as the timer arm body at :2270: expire
        // against the real, now-advanced clock — no synthetic timestamp.
        let expired = queue.expire_due_unresolved_barriers(std::time::Instant::now());
        assert!(
            !expired.is_empty(),
            "the real clock must have crossed the barrier deadline"
        );
        assert!(expired.contains(&ch));

        sync_dirty(&mut queue, &mut ledger);

        // Drive the same post-expiry dispatch the production arm calls at
        // :2277, proving B is claimed, not merely flushable.
        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![Some(agent)]);
        let ctx = test_prompt_context();
        let dispatched = dispatch_pending(&mut pool, &mut queue, &mut ledger, &ctx);

        assert_eq!(
            dispatched.len(),
            1,
            "B must dispatch once the barrier's real deadline has passed"
        );
        assert_eq!(dispatched[0].0, ch);
    }

    // ── Ledger persistence round-trip ────────────────────────────────────

    #[tokio::test]
    async fn test_boot_recover_ledger_commit_persists_to_disk() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let ev = make_channel_event(&keys, ch, "persist-test");
        let rec = ledger_record(&ev, 1, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![rec])]),
        };

        let (server, base_url) = mock_rest_server(vec![ev.clone()]).await;
        let rest = rest_client(&base_url);

        let dir = tempfile::tempdir().unwrap();
        let mut queue = EventQueue::new(DedupMode::Queue);
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        let (_ledger2, staged2) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        assert!(
            !staged2.channels.is_empty(),
            "committed data survives reload"
        );
        let records = staged2.channels.get(&ch).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].event_id, ev.id.to_hex());

        server.abort();
    }

    // ── F1: boot-recovery wake — dispatch occurs with no external event ──

    #[tokio::test]
    async fn test_boot_recover_dispatches_immediately_on_quiet_harness() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "boot-wake");
        let record_a = ledger_record(&event_a, 1, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![record_a])]),
        };

        let (server, base_url) = mock_rest_server(vec![event_a.clone()]).await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let mut ledger = Ledger::disabled();
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        // `has_flushable_work` is true on the old code too (boot_recover
        // always left work flushable — the defect was that nothing woke up
        // to *dispatch* it). Drive the actual production wake seam: the
        // pre-loop `dispatch_pending` call the main loop makes immediately
        // after `boot_recover` returns, with no external event, no
        // maintenance tick, and no unresolved barrier. Deleting that call
        // (reverting to F1-old) must fail this test.
        let agent = dummy_agent(0).await;
        let mut pool = AgentPool::from_slots(vec![Some(agent)]);
        let ctx = test_prompt_context();

        let dispatched = dispatch_pending(&mut pool, &mut queue, &mut ledger, &ctx);

        assert_eq!(
            dispatched.len(),
            1,
            "quiet-boot dispatch must claim the recovered channel's batch immediately"
        );
        assert_eq!(dispatched[0].0, ch);
        assert_eq!(
            pool.task_map().len(),
            1,
            "dispatch must spawn exactly one prompt task for the recovered batch"
        );
        assert!(
            !queue.has_flushable_work(),
            "the recovered batch must have been claimed, not left pending"
        );

        server.abort();
    }

    // ── F2: epoch promotion from full snapshot, not just fetched subset ──

    #[tokio::test]
    async fn test_boot_recover_promotion_from_full_snapshot_unfetched_promotes() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let ev_fetched = make_channel_event(&keys, ch, "fetched-B");
        let ev_unfetched = make_channel_event(&keys, ch, "unfetched-A");
        let rec_fetched = ledger_record(&ev_fetched, 2, "test");
        let rec_unfetched = ledger_record(&ev_unfetched, 1, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![rec_unfetched, rec_fetched])]),
        };

        let (server, base_url) = mock_rest_server(vec![ev_fetched.clone()]).await;
        let rest = rest_client(&base_url);

        let dir = tempfile::tempdir().unwrap();
        let mut queue = EventQueue::new(DedupMode::Queue);
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 1, "only fetched event in queue");
        assert!(
            triggers[0].cap_exempt,
            "fetched event must be promoted (no exempt in full snapshot)"
        );

        let unresolved = ledger.find_unresolved(ch, &ev_unfetched.id.to_hex());
        assert!(unresolved.is_some(), "unfetched must be unresolved");
        assert!(
            unresolved.unwrap().cap_exempt,
            "unresolved must also be promoted (whole-snapshot epoch rule)"
        );

        server.abort();
    }

    #[tokio::test]
    async fn test_boot_recover_promotion_preserves_class_when_exempt_exists() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let ev_exempt = make_channel_event(&keys, ch, "exempt-A");
        let ev_counted = make_channel_event(&keys, ch, "counted-B");
        let rec_exempt = ledger_record_exempt(&ev_exempt, 1, "test");
        let rec_counted = ledger_record(&ev_counted, 2, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![rec_exempt, rec_counted])]),
        };

        let (server, base_url) =
            mock_rest_server(vec![ev_exempt.clone(), ev_counted.clone()]).await;
        let rest = rest_client(&base_url);

        let dir = tempfile::tempdir().unwrap();
        let mut queue = EventQueue::new(DedupMode::Queue);
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 2);
        let exempt_trigger = triggers.iter().find(|t| t.admission_seq == 1).unwrap();
        let counted_trigger = triggers.iter().find(|t| t.admission_seq == 2).unwrap();
        assert!(
            exempt_trigger.cap_exempt,
            "exempt record must keep its class"
        );
        assert!(
            !counted_trigger.cap_exempt,
            "counted record must keep its class when exempt exists"
        );

        server.abort();
    }

    #[tokio::test]
    async fn test_boot_recover_promotion_unresolved_keeps_class_when_exempt_exists() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let ev_exempt = make_channel_event(&keys, ch, "exempt-A");
        let ev_counted = make_channel_event(&keys, ch, "counted-unfetched");
        let rec_exempt = ledger_record_exempt(&ev_exempt, 1, "test");
        let rec_counted = ledger_record(&ev_counted, 2, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![rec_exempt, rec_counted])]),
        };

        let (server, base_url) = mock_rest_server(vec![ev_exempt.clone()]).await;
        let rest = rest_client(&base_url);

        let dir = tempfile::tempdir().unwrap();
        let mut queue = EventQueue::new(DedupMode::Queue);
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        let unresolved = ledger.find_unresolved(ch, &ev_counted.id.to_hex());
        assert!(unresolved.is_some());
        assert!(
            !unresolved.unwrap().cap_exempt,
            "unresolved counted record must keep its class when exempt exists in snapshot"
        );

        server.abort();
    }

    // P2-F2(b) residual: an exempt record that the bridge OMITS (stays
    // unresolved) must not cause the fetched, non-exempt counterpart to be
    // promoted anyway. `boot_recover` decides promote_channels from the full
    // gated snapshot (both records: one exempt) — so the channel is NOT in
    // promote_channels, and the fetched counted record must reach
    // `import_recovered` still bearing `cap_exempt = false`. Before the
    // P2-F2(b) fix, `import_recovered` independently recomputed promotion
    // from only the events vector it received (just the fetched one),
    // saw zero exempt records in THAT subset, and promoted it anyway —
    // this is the exact defect Paul's residual review caught.
    #[tokio::test]
    async fn test_boot_recover_exempt_unfetched_does_not_promote_fetched_counted() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let ev_exempt_unfetched = make_channel_event(&keys, ch, "exempt-omitted-by-bridge");
        let ev_counted_fetched = make_channel_event(&keys, ch, "counted-fetched");
        let rec_exempt = ledger_record_exempt(&ev_exempt_unfetched, 1, "test");
        let rec_counted = ledger_record(&ev_counted_fetched, 2, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![rec_exempt, rec_counted])]),
        };

        // Bridge returns only the counted record — the exempt record is
        // omitted (stays unresolved), mirroring a partial chunk response.
        let (server, base_url) = mock_rest_server(vec![ev_counted_fetched.clone()]).await;
        let rest = rest_client(&base_url);

        let dir = tempfile::tempdir().unwrap();
        let mut queue = EventQueue::new(DedupMode::Queue);
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(
            triggers.len(),
            1,
            "only the fetched record reaches the queue"
        );
        assert!(
            !triggers[0].cap_exempt,
            "fetched counted record must stay counted — it must not be promoted \
             just because it was the only entry import_recovered's events vec saw"
        );

        // Round-trip through commit + reload: the persisted class for the
        // unresolved exempt record must also be untouched.
        let unresolved = ledger.find_unresolved(ch, &ev_exempt_unfetched.id.to_hex());
        assert!(unresolved.is_some());
        assert!(
            unresolved.unwrap().cap_exempt,
            "unresolved exempt record must keep its class"
        );

        server.abort();
    }

    // ── F3: barrier deadline survives fetch latency ─────────────────────

    // Not `start_paused`: the barrier deadline is `std::time::Instant`
    // (real wall clock), but the fetch-latency simulation lives inside the
    // mock server's tokio timer. Under a paused virtual clock the two
    // would never interact — `std::time::Instant::now()` cannot observe a
    // virtual-clock advance — so this test needs a real sleep to actually
    // race fetch latency against the barrier deadline, same as the
    // large-ledger hanging-bridge test above.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_boot_recover_barrier_deadline_not_consumed_by_fetch() {
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let ev_fetched = make_channel_event(&keys, ch, "fetched-B");
        let ev_unfetched = make_channel_event(&keys, ch, "unfetched-A");
        let rec_fetched = ledger_record(&ev_fetched, 2, "test");
        let rec_unfetched = ledger_record(&ev_unfetched, 1, "test");

        let staged = StagedLedger {
            channels: HashMap::from([(ch, vec![rec_unfetched, rec_fetched])]),
        };

        // Real 50s delay inside the mock server's request handler — the
        // fetch loop is genuinely awaiting the response for 50 of the 60s
        // fetch budget when boot_recover reaches barrier registration.
        let (server, base_url) =
            mock_rest_server_delayed(vec![ev_fetched.clone()], Duration::from_secs(50)).await;
        let rest = rest_client(&base_url);

        let mut queue = EventQueue::new(DedupMode::Queue);
        let mut ledger = Ledger::disabled();
        let live_channels = HashSet::from([ch]);
        let mut suppression = HashSet::new();

        boot_recover(
            &mut queue,
            &mut ledger,
            staged,
            &live_channels,
            &rest,
            &mut suppression,
        )
        .await;

        let deadline = queue.next_unresolved_barrier_deadline();
        assert!(deadline.is_some(), "barrier must be armed for unfetched A");

        let remaining = deadline
            .unwrap()
            .saturating_duration_since(std::time::Instant::now());
        assert!(
            remaining >= Duration::from_secs(55),
            "barrier must have nearly the full 60s window remaining (got {remaining:?}), \
             not be consumed by the 50s fetch latency"
        );

        server.abort();
    }

    // ── P3-F1: barrier-aware skip_steer ─────────────────────────────────

    #[tokio::test]
    async fn test_live_event_skips_steer_while_barrier_armed() {
        // P3-F1 fix: a fresh live event for a channel that has an active
        // unresolved barrier must return skip_steer = true (queue normally
        // and wait behind the hole) rather than bypassing it through native
        // steer.
        //
        // Red-on-old: before the fix, `admit_live_event`'s ordinary branch
        // always returned `(ok, false)` — skip_steer was never set for
        // ordinary pushes, so the assertion `skip_steer == true` failed.
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_c = make_channel_event(&keys, ch, "fresh-C");

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let mut suppression: HashSet<String> = HashSet::new();

        // Arm a barrier for `ch` — simulates boot_recover registering an
        // unfetched hole at seq 1.
        let seqs = std::collections::BTreeSet::from([1u64]);
        let deadline = Instant::now() + Duration::from_secs(60);
        queue.set_unresolved_barrier(ch, seqs, deadline);

        // Fresh live event C arrives above the hole.
        let (accepted, skip_steer) = admit_live_event(
            &mut queue,
            &mut ledger,
            &mut suppression,
            ch,
            event_c.clone(),
            "mention".into(),
        );

        assert!(
            accepted,
            "fresh event above barrier must be admitted into queue"
        );
        assert!(
            skip_steer,
            "fresh live event must skip native steer while the channel's barrier is armed"
        );
    }

    #[tokio::test]
    async fn test_live_event_steers_after_barrier_expires() {
        // Companion: once the barrier has been cleared (expired or all
        // unresolved seqs resolved), ordinary live events must NOT skip steer.
        //
        // Closes the mutation gap where always returning skip_steer = true
        // would slip through — this verifies the "no barrier → steer allowed"
        // half of the invariant.
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "fresh-D");

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let mut suppression: HashSet<String> = HashSet::new();

        // Arm and immediately expire the barrier.
        let seqs = std::collections::BTreeSet::from([1u64]);
        let past_deadline = Instant::now() - Duration::from_secs(1);
        queue.set_unresolved_barrier(ch, seqs, past_deadline);
        queue.expire_due_unresolved_barriers(Instant::now());
        assert!(
            !queue.has_active_unresolved_barrier(ch),
            "barrier must be cleared before the assertion below"
        );

        let (accepted, skip_steer) = admit_live_event(
            &mut queue,
            &mut ledger,
            &mut suppression,
            ch,
            event_a.clone(),
            "mention".into(),
        );

        assert!(accepted, "ordinary live event must be admitted");
        assert!(
            !skip_steer,
            "ordinary live event must take the native steer path when no barrier is armed"
        );
    }

    // ── P3-F2: sync-before-exit ─────────────────────────────────────────

    #[tokio::test]
    async fn test_exit_via_handle_prompt_result_syncs_ledger_before_break() {
        // P3-F2 model: this test is a NON-DISCRIMINATING model of the break-
        // site contract, not a red-on-old mutation of production code.
        //
        // It models the fixed main-loop path — dispatch in-flight, force Exit
        // from handle_prompt_result, explicitly call sync_dirty (mirroring what
        // each of the three fixed break sites now does), then reload — to
        // confirm the zero-triggers post-condition holds when sync_dirty IS
        // called. Because sync_dirty is called in the test body itself, removing
        // any of the three production sync_dirty calls does not make this test
        // fail; source review of the three break sites (lib.rs ~2290, ~2308,
        // ~2336) is the discriminating verification for P3-F2.
        //
        // Contract: after a circuit-open exit path that calls sync_dirty, the
        // reloaded ledger must contain zero recovered triggers (the terminated
        // in-flight work must not resurrect).
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "msg");

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);

        // Dispatch a real in-flight batch.
        let mut pool = dispatch_one_in_flight(&mut queue, &mut ledger, ch, &event_a).await;
        assert!(
            !queue.recoverable_triggers(ch).is_empty(),
            "setup: in-flight trigger must be present"
        );

        // Force the circuit open.
        let config = test_config();
        let mut crash_history = vec![SlotCircuit {
            crash_times: Vec::new(),
            open_until: None,
            respawn_in_flight: false,
        }];
        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            crash_history[0].record_crash();
        }
        assert!(
            matches!(crash_history[0].record_crash(), CrashVerdict::CircuitOpen),
            "setup: circuit must be open"
        );

        let agent = dummy_agent(0).await;
        let (respawn_tx, _rx) = mpsc::channel(8);
        let mut respawn_tasks = tokio::task::JoinSet::new();
        let result = PromptResult {
            agent,
            source: PromptSource::Channel(ch),
            turn_id: "t".into(),
            outcome: PromptOutcome::Timeout(TimeoutKind::Hard {
                recently_active: false,
            }),
            batch: None,
        };

        let mut heartbeat_in_flight = false;
        let removed_channels = HashSet::new();

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
            None,
            None,
        );

        assert!(
            action == LoopAction::Exit,
            "setup: circuit-open exit must return Exit"
        );

        // The fix: sync_dirty called before break (P3-F2).
        sync_dirty(&mut queue, &mut ledger);

        let (_, staged) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        assert!(
            staged.channels.is_empty(),
            "after circuit-open exit with sync, reload must find zero recovered triggers (got: {:?})",
            staged.channels
        );
    }

    // ── P3-F3: persist returns success; last_written advances only on success

    #[test]
    fn test_failed_persist_does_not_advance_last_written_allowing_retry() {
        // P3-F3 fix: when persist fails (temp-file path blocked),
        // `last_written` must NOT be advanced. The skip-identical check
        // (P2-F3) compares against `last_written`; if a failed write is
        // recorded as success, the healing retry is silently skipped.
        //
        // Red-on-old: sync() advanced last_written before calling persist(),
        // so the second identical sync() was suppressed even though the first
        // write never reached disk.
        let ch = Uuid::new_v4();
        let keys = Keys::generate();
        let event_a = make_channel_event(&keys, ch, "msg");

        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);

        // Block the .tmp path — derive the actual ledger path from the loaded
        // ledger so the filename (pubkey16 + relay hash) stays in sync automatically.
        let ledger_path = ledger.path().unwrap().to_path_buf();
        let tmp_path = {
            let mut s = ledger_path.as_os_str().to_os_string();
            s.push(".tmp");
            std::path::PathBuf::from(s)
        };
        std::fs::create_dir_all(&tmp_path).expect("create blocker dir");

        let trigger = crate::queue::RecoverableTrigger {
            event_id: event_a.id.to_hex(),
            prompt_tag: "mention".into(),
            admission_seq: 1,
            enqueued_at_unix: 1000,
            cap_exempt: false,
        };

        // First sync: write fails (path is a directory).
        ledger.sync(ch, vec![trigger.clone()]);

        // Remove blocker, retry with identical data — must succeed now.
        std::fs::remove_dir_all(&tmp_path).expect("remove blocker");
        ledger.sync(ch, vec![trigger.clone()]);

        let (_, staged) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        assert_eq!(
            staged.channels.len(),
            1,
            "after unblocked retry sync, ledger file must contain the channel record"
        );
        let records = staged.channels.get(&ch).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].event_id, event_a.id.to_hex());
    }

    // ── P3-F3 (invalidate_channel): failed persist → different channel sync → removed channel absent

    #[test]
    fn test_invalidate_channel_failed_persist_heals_on_subsequent_sync() {
        // P3-F3 regression guard for invalidate_channel: when the invalidation
        // write fails (dirty flag set), a later successful sync of a DIFFERENT
        // channel must write the full map WITHOUT the removed channel, healing
        // the file.
        //
        // Red-on-old (advance-on-success): invalidate_channel kept the removed
        // channel in last_written on persist failure; subsequent syncs wrote it
        // back, permanently losing the removal intent until reboot.
        let ch_keep = Uuid::new_v4();
        let ch_remove = Uuid::new_v4();

        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);

        // Write both channels to disk; derive path from the ledger so the
        // filename (pubkey16 + relay hash) stays in sync automatically.
        let ledger_path = ledger.path().unwrap().to_path_buf();
        let tmp_path = {
            let mut s = ledger_path.as_os_str().to_os_string();
            s.push(".tmp");
            std::path::PathBuf::from(s)
        };

        let trigger_keep = crate::queue::RecoverableTrigger {
            event_id: "keep-event".into(),
            prompt_tag: "mention".into(),
            admission_seq: 1,
            enqueued_at_unix: 1000,
            cap_exempt: false,
        };
        let trigger_remove = crate::queue::RecoverableTrigger {
            event_id: "remove-event".into(),
            prompt_tag: "mention".into(),
            admission_seq: 2,
            enqueued_at_unix: 1001,
            cap_exempt: false,
        };
        ledger.sync(ch_keep, vec![trigger_keep.clone()]);
        ledger.sync(ch_remove, vec![trigger_remove]);
        assert!(ledger_path.exists(), "setup: both channels written");

        // Block the .tmp path so the invalidation write fails.
        std::fs::create_dir_all(&tmp_path).expect("create blocker dir");
        ledger.invalidate_channel(ch_remove);
        // The persist failed — but last_written must already reflect the removal.

        // Unblock and sync a DIFFERENT channel — this must write the full map
        // (dirty flag forces a write even though ch_keep's content is unchanged).
        std::fs::remove_dir_all(&tmp_path).expect("remove blocker");
        ledger.sync(ch_keep, vec![trigger_keep.clone()]);

        // Reload: removed channel must be absent.
        let (_, staged) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        assert!(
            !staged.channels.contains_key(&ch_remove),
            "after invalidation + failed persist + different-channel sync, removed channel must be absent on reload (got: {:?})",
            staged.channels,
        );
        assert!(
            staged.channels.contains_key(&ch_keep),
            "kept channel must still be present"
        );
    }

    // ── P3-F4: persisted prompt_tag through from_recovered ───────────────

    #[tokio::test]
    async fn test_unresolved_resolution_uses_persisted_prompt_tag_not_live_tag() {
        // P3-F4 fix: when a live event resolves an unresolved ledger record,
        // the recovered QueuedEvent must use the *persisted* prompt_tag from
        // the LedgerRecord, not the live relay delivery's tag.
        //
        // Red-on-old: admit_live_event passed the live `prompt_tag` argument
        // to from_recovered — a live delivery with a different tag would
        // silently override the persisted one.
        let keys = Keys::generate();
        let ch = Uuid::new_v4();
        let event_a = make_channel_event(&keys, ch, "msg");

        let record_a = LedgerRecord {
            event_id: event_a.id.to_hex(),
            prompt_tag: "original".into(),
            admission_seq: 1,
            enqueued_at_unix: 1001,
            cap_exempt: false,
        };

        let mut queue = EventQueue::new(DedupMode::Queue);
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _) = Ledger::load(dir.path(), "test_pubkey", "ws://localhost:3000", 0);
        let mut suppression: HashSet<String> = HashSet::new();

        ledger.add_unresolved(ch, record_a);

        // Live delivery arrives with a different tag.
        let (accepted, skip_steer) = admit_live_event(
            &mut queue,
            &mut ledger,
            &mut suppression,
            ch,
            event_a.clone(),
            "new".into(), // live delivery tag — must NOT win
        );

        assert!(accepted, "resolving event must be admitted");
        assert!(skip_steer, "resolving event skips native steer");

        let triggers = queue.recoverable_triggers(ch);
        assert_eq!(triggers.len(), 1);
        assert_eq!(
            triggers[0].prompt_tag, "original",
            "recovered trigger must retain the persisted prompt_tag, not the live-delivery tag"
        );
    }
}
