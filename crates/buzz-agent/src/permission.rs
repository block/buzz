//! `session/request_permission` broker.
//!
//! buzz-agent asks the client to authorize every LLM-issued MCP tool call
//! *before* executing it; the client applies `BUZZ_ACP_PERMISSION_POLICY` and
//! answers. The agent never reads the policy — it always asks, matching the
//! layering of every other ACP harness. This module owns the whole request
//! correlation lifecycle so the rest of the agent only sees a single
//! `Allowed`/`Denied`/`Cancelled` decision.
//!
//! ## Invariants
//!
//! - **Process-wide admission.** The broker owns a global [`Semaphore`]
//!   (`BUZZ_AGENT_MAX_PENDING_PERMISSIONS`) acquired *before* any correlation
//!   entry is inserted. The per-turn `execute_parallel` semaphore is fresh per
//!   turn and sessions are unbounded by default, so only this global cap bounds
//!   simultaneously outstanding asks process-wide.
//! - **Abort-safe cleanup.** A successful admission returns a
//!   [`PendingPermission`] lease that owns the admission permit and the
//!   correlation id. Its `Drop` synchronously removes the still-pending entry
//!   and releases the slot, covering task abort/panic that bypasses the normal
//!   `run_prompt` tail.
//! - **Claim-before-wake / at-most-once.** [`PermissionBroker::deliver`] removes
//!   the entry *before* waking the waiter, so each id resolves at most once and
//!   a later lease `Drop` is a harmless no-op.
//! - **Unknown/late ids ignored.** A response whose id is not a live entry is
//!   logged and dropped.
//! - **Undeliverable asks are terminal.** If the output wire is closed when the
//!   request is enqueued, [`PermissionBroker::request_permission`] fails closed
//!   immediately (dropping the lease removes the entry and releases the permit)
//!   rather than leaving a resident waiter to time out — a closed wire can never
//!   carry the reply.
//! - **Single absolute deadline.** Admission wait and response wait share one
//!   absolute deadline computed at gate entry, so a saturated call cannot live
//!   for two full timeout windows.
//! - **Cancellation races inside the wait.** The waiter selects on the turn's
//!   cancel receiver directly; resolution never depends on the outer abort
//!   drain.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{oneshot, watch, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use crate::types::ToolCall;
use crate::wire::{self, WireSender, ALLOW_OPTION_ID};

/// Model-visible tool error when a call is not authorized. Rides the normal
/// tool-failure path so the turn continues, matching every other tool error.
pub const PERMISSION_DENIED_MSG: &str = "permission denied: the tool call was not authorized";

/// Model-visible tool error when the client never answers within the deadline.
pub const PERMISSION_TIMEOUT_MSG: &str =
    "permission request timed out: the tool call was not authorized";

/// Model-visible tool error when the permission request cannot be delivered
/// because the output wire is closed. Terminal and immediate — no waiter is
/// left resident, since a closed wire can never carry a reply.
pub const PERMISSION_WIRE_CLOSED_MSG: &str =
    "permission request undeliverable: the tool call was not authorized";

/// Outcome of asking the client to authorize one tool call.
#[derive(Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    /// The client selected the offered allow option — execute the tool.
    Allowed,
    /// Every non-authorizing shape (reject, cancelled outcome, JSON-RPC error,
    /// malformed response, unknown outcome, wrong/unknown optionId, timeout,
    /// wire-channel closure). Fails closed with the given model-visible reason;
    /// the turn continues.
    Denied(&'static str),
    /// The turn was cancelled while admitting or waiting. No tool runs and the
    /// caller propagates cancellation exactly as the existing cancel path does.
    Cancelled,
}

/// Broker owned by `App` for the connection lifetime.
pub struct PermissionBroker {
    /// Global admission cap. Acquired before any entry is inserted.
    sem: Arc<Semaphore>,
    /// Live correlation entries: outbound request id -> response sender.
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    /// Monotonic id allocator. Never reused within a process lifetime, so a
    /// late response for a removed id can never collide with a fresh request.
    next_id: AtomicU64,
    /// Absolute deadline budget shared by admission + response wait.
    timeout: Duration,
    /// Test-only: invoked by the waiter the instant it observes a delivered
    /// response, with `true` iff the entry was already claimed (removed) from
    /// `pending` before the wake. Makes claim-before-wake ordering
    /// mutation-sensitive — a wake-before-claim mutant reports `false`, which a
    /// purely behavioral test cannot detect (the waiter reads the oneshot once
    /// either way).
    #[cfg(test)]
    wake_observer: Mutex<Option<WakeObserver>>,
}

/// Test-only wake-boundary observer; see [`PermissionBroker::wake_observer`].
#[cfg(test)]
type WakeObserver = Arc<dyn Fn(bool) + Send + Sync>;

impl PermissionBroker {
    /// `max_pending` is validated `>= 1` by config; `timeout` is injectable so
    /// broker unit tests exercise the timeout/abort paths without a 330s wait.
    pub fn new(max_pending: usize, timeout: Duration) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(max_pending.max(1))),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(0),
            timeout,
            #[cfg(test)]
            wake_observer: Mutex::new(None),
        }
    }

    /// Test-only: register a callback the waiter fires the instant it observes a
    /// delivered response, with `true` iff the correlation entry was already
    /// claimed (removed) before the wake. Used to prove claim-before-wake
    /// ordering in a way a wake-before-claim mutant cannot satisfy.
    #[cfg(test)]
    pub fn set_wake_observer(&self, observer: WakeObserver) {
        *self.wake_observer.lock().unwrap() = Some(observer);
    }

    /// Test-only: fire the wake observer (if any) with the claimed-before-wake
    /// status of `id`. Called synchronously by the waiter the moment it receives
    /// its response, so the observed `pending` state is exactly the state at the
    /// wake — deterministic in production (removal happens-before the send) and
    /// violated by a wake-before-claim mutant.
    #[cfg(test)]
    fn observe_wake(&self, id: u64) {
        let claimed = !self.pending.lock().unwrap().contains_key(&id);
        let observer = self.wake_observer.lock().unwrap().clone();
        if let Some(observer) = observer {
            observer(claimed);
        }
    }

    /// Number of live (unresolved, un-dropped) correlation entries. Test-only
    /// observability for the drop-guard and delivery invariants.
    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap().len()
    }

    /// Free admission slots. Test-only, so a test can prove a terminal path
    /// (delivery, timeout, cancel, drop) actually released the capacity it
    /// held rather than leaking it.
    #[cfg(test)]
    pub fn available_permits(&self) -> usize {
        self.sem.available_permits()
    }

    /// Deliver a client response to its waiter. Claims (removes) the entry
    /// before waking so the id resolves at most once; unknown/late ids are
    /// logged and ignored. `result` is the JSON-RPC `result` field (or
    /// `Value::Null` for an error/malformed response — every such shape fails
    /// the authorization predicate and denies).
    pub fn deliver(&self, id: &Value, result: Value) {
        let Some(key) = parse_id(id) else {
            tracing::debug!(target: "permission", "ignoring response with unrecognized id {id}");
            return;
        };
        // Claim before wake: remove first, then send into the removed sender.
        let sender = self.pending.lock().unwrap().remove(&key);
        match sender {
            Some(tx) => {
                // The receiver may already be gone (waiter cancelled/timed out
                // and dropped the lease); a failed send is a harmless no-op.
                let _ = tx.send(result);
            }
            None => {
                tracing::debug!(target: "permission", "ignoring unknown/late permission id {id}");
            }
        }
    }

    /// Ask the client to authorize `call`, returning the decision.
    ///
    /// Sequence: acquire global admission (racing cancel + deadline) → insert
    /// correlation entry (held by an abort-safe lease) → send the version-aware
    /// request → wait for the response (racing cancel + deadline). One absolute
    /// deadline bounds both waits.
    pub async fn request_permission(
        &self,
        wire: &WireSender,
        version: u32,
        session_id: &str,
        call: &ToolCall,
        cancel: &mut watch::Receiver<bool>,
    ) -> PermissionDecision {
        let deadline = Instant::now() + self.timeout;

        // ── Admission ──────────────────────────────────────────────────────
        // Early cancel check: watch::changed() only fires on NEW writes.
        if *cancel.borrow() {
            return PermissionDecision::Cancelled;
        }
        let permit = tokio::select! {
            biased;
            _ = cancel.changed() => return PermissionDecision::Cancelled,
            _ = tokio::time::sleep_until(deadline) => {
                return PermissionDecision::Denied(PERMISSION_TIMEOUT_MSG);
            }
            p = Arc::clone(&self.sem).acquire_owned() => match p {
                Ok(p) => p,
                // Semaphore is never closed in production; treat as fail-closed.
                Err(_) => return PermissionDecision::Denied(PERMISSION_DENIED_MSG),
            },
        };

        // Insert the correlation entry under the owned permit. The lease's Drop
        // removes the entry + releases the slot on every exit path below,
        // including task abort.
        let mut lease = self.register(permit);

        // ── Send the version-aware request ─────────────────────────────────
        let params = wire::request_permission_params(
            version,
            session_id,
            &call.provider_id,
            &call.name,
            &call.arguments,
        );
        // ── Send (deadline- and cancel-governed) ───────────────────────────
        // Enqueue is the third phase under the single absolute deadline. A
        // full-but-live channel makes `send_checked` wait for capacity; racing
        // it against cancel + the deadline means a stalled writer cannot hold
        // the ask (and its global permit) past the advertised deadline, and
        // `session/cancel` resolves it promptly. On send error the wire is
        // closed: fail closed at once — dropping `lease` removes the entry and
        // releases the permit synchronously.
        let request = wire::request_permission(lease.id_value.clone(), params);
        tokio::select! {
            biased;
            _ = cancel.changed() => return PermissionDecision::Cancelled,
            _ = tokio::time::sleep_until(deadline) => {
                return PermissionDecision::Denied(PERMISSION_TIMEOUT_MSG);
            }
            r = wire::send_checked(wire, request) => {
                if r.is_err() {
                    return PermissionDecision::Denied(PERMISSION_WIRE_CLOSED_MSG);
                }
            }
        }

        // ── Response wait ──────────────────────────────────────────────────
        if *cancel.borrow() {
            return PermissionDecision::Cancelled;
        }
        #[cfg(test)]
        let id = lease.id;
        tokio::select! {
            biased;
            _ = cancel.changed() => PermissionDecision::Cancelled,
            r = &mut lease.rx => match r {
                Ok(result) => {
                    // The waiter observes delivery here. At this instant the
                    // entry must already be claimed (removed) — delivery removes
                    // before it sends. The observer is test-only and a no-op in
                    // production.
                    #[cfg(test)]
                    self.observe_wake(id);
                    evaluate(&result)
                }
                // Sender dropped without sending — should not happen (delivery
                // always sends before drop); fail closed.
                Err(_) => PermissionDecision::Denied(PERMISSION_DENIED_MSG),
            },
            _ = tokio::time::sleep_until(deadline) => {
                PermissionDecision::Denied(PERMISSION_TIMEOUT_MSG)
            }
        }
        // `lease` drops here: entry removed (no-op if delivered) + slot released.
    }

    /// Allocate an id, insert its response sender, and return the abort-safe
    /// lease holding the receiver + owned permit.
    fn register(&self, permit: OwnedSemaphorePermit) -> PendingPermission {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        PendingPermission {
            id,
            id_value: Value::String(format!("perm-{id}")),
            rx,
            pending: Arc::clone(&self.pending),
            _permit: permit,
        }
    }
}

/// Abort-safe correlation lease. Owns the admission permit and the correlation
/// id; its `Drop` synchronously removes the still-pending entry and releases
/// the slot. Delivery removes the entry first, so a later drop is a no-op.
struct PendingPermission {
    id: u64,
    id_value: Value,
    rx: oneshot::Receiver<Value>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for PendingPermission {
    fn drop(&mut self) {
        // Synchronous, non-async removal — safe from a Drop and required for
        // abort/panic paths. No-op if delivery already claimed the entry.
        self.pending.lock().unwrap().remove(&self.id);
        // `_permit` drops → global admission slot released.
    }
}

/// The authorization predicate, stated once: execute IFF the client selected an
/// option AND the selected `optionId` equals exactly this request's offered
/// allow-option id. Every other shape fails closed.
fn evaluate(result: &Value) -> PermissionDecision {
    let outcome = &result["outcome"];
    if outcome["outcome"] == "selected" && outcome["optionId"].as_str() == Some(ALLOW_OPTION_ID) {
        PermissionDecision::Allowed
    } else {
        PermissionDecision::Denied(PERMISSION_DENIED_MSG)
    }
}

/// Recover the correlation key from an outbound request id echoed by the
/// client. Only ids we minted (`perm-<n>`, canonical decimal) are ours; a
/// noncanonical alias (`perm-01`, `perm-+0`, `perm-00`) or any other string is
/// a foreign/stale id and is ignored. Requiring an exact round-trip means only
/// the string the broker actually minted correlates — no alias is ever live.
fn parse_id(id: &Value) -> Option<u64> {
    let s = id.as_str()?;
    let n: u64 = s.strip_prefix("perm-")?.parse().ok()?;
    (format!("perm-{n}") == s).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::AsyncWrite;
    use tokio::sync::mpsc;

    const LONG: Duration = Duration::from_secs(30);
    const SHORT: Duration = Duration::from_millis(60);

    fn tool_call() -> ToolCall {
        ToolCall {
            provider_id: "fake".into(),
            name: "fake__shell".into(),
            arguments: json!({ "command": "ls" }),
            provider_extra: serde_json::Map::new(),
        }
    }

    fn selected(option_id: &str) -> Value {
        json!({ "outcome": { "outcome": "selected", "optionId": option_id } })
    }

    /// Pull the next outbound frame off the wire and return its JSON-RPC `id`.
    /// Reading it also proves the request was registered and sent (delivery
    /// only happens after `register`).
    async fn next_request_id(rx: &mut mpsc::Receiver<wire::WireMsg>) -> Value {
        let wire::WireMsg::Notify(v) = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("a request frame")
            .expect("wire open");
        assert_eq!(v["method"], "session/request_permission");
        v["id"].clone()
    }

    /// An `AsyncWrite` that accepts every write but fails on `flush`, modelling
    /// Tokio's blocking stdout when the pipe has broken: `write_all` reports
    /// `Ok` (the underlying blocking write is only scheduled) and the real
    /// error surfaces at `flush`. Used to prove the writer treats flush failure
    /// as connection-fatal.
    struct FlushFailSink;

    impl AsyncWrite for FlushFailSink {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe)))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    // ── Authorization predicate (fail-closed) ────────────────────────────────

    #[test]
    fn test_selected_allow_option_authorizes() {
        assert_eq!(
            evaluate(&selected(ALLOW_OPTION_ID)),
            PermissionDecision::Allowed
        );
    }

    #[test]
    fn test_every_non_allow_shape_denies() {
        // reject, wrong/unknown option id, unknown outcome, missing fields,
        // empty object — the full adversarial set the predicate must reject.
        let denied = [
            selected("reject_once"),
            selected("some_unknown_option"),
            json!({ "outcome": { "outcome": "cancelled" } }),
            json!({ "outcome": { "outcome": "selected" } }), // no optionId
            json!({ "outcome": { "outcome": "banana", "optionId": ALLOW_OPTION_ID } }),
            json!({ "outcome": {} }),
            json!({}),
            Value::Null,
        ];
        for shape in denied {
            assert_eq!(
                evaluate(&shape),
                PermissionDecision::Denied(PERMISSION_DENIED_MSG),
                "shape must fail closed: {shape}"
            );
        }
    }

    // ── Id correlation ───────────────────────────────────────────────────────

    #[test]
    fn test_parse_id_accepts_only_minted_ids() {
        assert_eq!(parse_id(&json!("perm-0")), Some(0));
        assert_eq!(parse_id(&json!("perm-42")), Some(42));
        assert_eq!(parse_id(&json!("perm-x")), None);
        assert_eq!(parse_id(&json!("42")), None); // foreign numeric-string id
        assert_eq!(parse_id(&json!(42)), None); // foreign numeric id
        assert_eq!(parse_id(&Value::Null), None);
    }

    /// Noncanonical strings that `u64::parse` would otherwise accept as aliases
    /// of a minted id must NOT correlate. Only the exact string the broker
    /// minted (`format!("perm-{n}")`) is live; leading zeros, a sign, or
    /// whitespace make the id foreign and it is ignored. Without the exact
    /// round-trip check these would resolve live asks under ids the broker
    /// never issued.
    #[test]
    fn test_parse_id_rejects_noncanonical_aliases() {
        for alias in [
            "perm-00",    // extra leading zero
            "perm-01",    // leading zero
            "perm-+0",    // explicit sign
            "perm-0x1",   // hex
            "perm- 1",    // leading space
            "perm-1 ",    // trailing space
            "perm-1_000", // digit separator
        ] {
            assert_eq!(
                parse_id(&json!(alias)),
                None,
                "alias must be foreign: {alias}"
            );
        }
    }

    // ── Delivery: exact allow / deny ──────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_deliver_allow_authorizes_and_frees_slot() {
        let broker = Arc::new(PermissionBroker::new(4, LONG));
        let (tx, mut rx) = mpsc::channel(8);
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);

        let b = Arc::clone(&broker);
        let call = tool_call();
        let task = tokio::spawn(async move {
            b.request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx)
                .await
        });

        let id = next_request_id(&mut rx).await;
        assert_eq!(broker.pending_count(), 1);
        assert_eq!(broker.available_permits(), 3);

        broker.deliver(&id, selected(ALLOW_OPTION_ID));
        assert_eq!(task.await.unwrap(), PermissionDecision::Allowed);
        assert_eq!(broker.pending_count(), 0, "entry claimed on delivery");
        assert_eq!(
            broker.available_permits(),
            4,
            "slot released after decision"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_deliver_reject_denies_and_frees_slot() {
        let broker = Arc::new(PermissionBroker::new(4, LONG));
        let (tx, mut rx) = mpsc::channel(8);
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);

        let b = Arc::clone(&broker);
        let call = tool_call();
        let task = tokio::spawn(async move {
            b.request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx)
                .await
        });

        let id = next_request_id(&mut rx).await;
        broker.deliver(&id, selected("reject_once"));
        assert_eq!(
            task.await.unwrap(),
            PermissionDecision::Denied(PERMISSION_DENIED_MSG)
        );
        assert_eq!(broker.pending_count(), 0);
        assert_eq!(broker.available_permits(), 4);
    }

    // ── Malformed response frames deny (Carl's review) ────────────────────────

    /// Route Carl's frame through the real `classify` → `deliver` path against a
    /// live waiter and assert the tool is denied. Delivers on the exact id the
    /// broker minted, so the only reason the waiter denies is that `classify`
    /// refused to forward the ambiguous/malformed `result`. `provider_id`
    /// carries which shape is under test so a failure names the mutant.
    async fn assert_malformed_frame_denies(provider_id: &str, frame: Value) {
        let broker = Arc::new(PermissionBroker::new(4, LONG));
        let (tx, mut rx) = mpsc::channel(8);
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);

        let b = Arc::clone(&broker);
        let call = tool_call();
        let task = tokio::spawn(async move {
            b.request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx)
                .await
        });

        let id = next_request_id(&mut rx).await;
        // Stamp the broker's minted id onto Carl's frame, then classify it
        // exactly as the dispatch loop would before handing `result` to deliver.
        let mut frame = frame;
        frame["id"] = id.clone();
        match crate::wire::classify(&frame) {
            crate::wire::Inbound::Response { id, result } => broker.deliver(&id, result),
            other => panic!("[{provider_id}] expected Response, got {other:?}"),
        }

        assert_eq!(
            task.await.unwrap(),
            PermissionDecision::Denied(PERMISSION_DENIED_MSG),
            "[{provider_id}] malformed frame must deny, not authorize the tool",
        );
        assert_eq!(broker.pending_count(), 0);
        assert_eq!(broker.available_permits(), 4);
    }

    /// Carl frame #1: `result` (well-formed `selected`/`allow_once`) AND `error`
    /// both present. The tool must not run — the ambiguous frame denies.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_frame_with_result_and_error_denies_tool() {
        assert_malformed_frame_denies(
            "result+error",
            json!({
                "jsonrpc": "2.0",
                "result": { "outcome": { "outcome": "selected", "optionId": ALLOW_OPTION_ID } },
                "error": { "code": -32603, "message": "internal" },
            }),
        )
        .await;
    }

    /// Carl frame #2: present non-string `method: 7` alongside a well-formed
    /// `selected` `result`. It is not a valid response — the tool must not run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_frame_with_non_string_method_denies_tool() {
        assert_malformed_frame_denies(
            "non-string-method",
            json!({
                "jsonrpc": "2.0",
                "method": 7,
                "result": { "outcome": { "outcome": "selected", "optionId": ALLOW_OPTION_ID } },
            }),
        )
        .await;
    }

    // ── Stale / unknown id ignored ────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_unknown_id_does_not_unblock_waiter() {
        let broker = Arc::new(PermissionBroker::new(4, LONG));
        let (tx, mut rx) = mpsc::channel(8);
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);

        let b = Arc::clone(&broker);
        let call = tool_call();
        let task = tokio::spawn(async move {
            b.request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx)
                .await
        });

        let real_id = next_request_id(&mut rx).await;
        // A stale/foreign id is dropped; the live entry survives.
        broker.deliver(&json!("perm-999"), selected(ALLOW_OPTION_ID));
        broker.deliver(&json!(1), selected(ALLOW_OPTION_ID));
        broker.deliver(&Value::Null, selected(ALLOW_OPTION_ID));
        assert_eq!(broker.pending_count(), 1, "waiter still pending");

        // The correct id resolves it exactly once.
        broker.deliver(&real_id, selected(ALLOW_OPTION_ID));
        assert_eq!(task.await.unwrap(), PermissionDecision::Allowed);
        assert_eq!(broker.pending_count(), 0);
    }

    // ── Timeout ───────────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_timeout_denies_and_removes_state() {
        let broker = Arc::new(PermissionBroker::new(4, SHORT));
        let (tx, _rx) = mpsc::channel(8);
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);
        let call = tool_call();

        // No delivery ever arrives: the shared deadline denies.
        let decision = broker
            .request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx)
            .await;
        assert_eq!(decision, PermissionDecision::Denied(PERMISSION_TIMEOUT_MSG));
        assert_eq!(
            broker.pending_count(),
            0,
            "timeout removes correlation state"
        );
        assert_eq!(broker.available_permits(), 4, "timeout releases the slot");
    }

    // ── Undeliverable ask (closed wire) is terminal ───────────────────────────

    /// When the output wire is closed, the ask can never be written and no
    /// reply can ever arrive. `request_permission` must fail closed
    /// *immediately* — denying with the wire-closed reason and leaving zero
    /// pending entries and zero held permits — rather than registering an entry
    /// that waits out the full deadline. Uses a LONG timeout so a wrong
    /// implementation that waits the deadline would visibly hang the test far
    /// past its own assertions.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_closed_wire_denies_immediately_without_leaking_state() {
        let broker = Arc::new(PermissionBroker::new(4, LONG));
        // Drop the receiver so every send fails: the writer is gone.
        let (tx, rx) = mpsc::channel(8);
        drop(rx);
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);
        let call = tool_call();

        // Bound the whole call: correct behavior returns at once; a regression
        // that waits the deadline blows this timeout instead of hanging LONG.
        let decision = tokio::time::timeout(
            Duration::from_secs(2),
            broker.request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx),
        )
        .await
        .expect("closed wire must deny immediately, not wait the deadline");

        assert_eq!(
            decision,
            PermissionDecision::Denied(PERMISSION_WIRE_CLOSED_MSG)
        );
        assert_eq!(
            broker.pending_count(),
            0,
            "undeliverable ask leaves no resident entry"
        );
        assert_eq!(
            broker.available_permits(),
            4,
            "undeliverable ask releases its admission slot"
        );
    }

    // ── Writer flush failure is connection-fatal ──────────────────────────────

    /// A blocking stdout can report `Ok` from `write_all` (the underlying
    /// blocking write is only scheduled) and surface the real error at `flush`.
    /// The writer must therefore treat flush failure exactly like write failure
    /// — return, dropping its receiver — so the connection supervisor observes
    /// writer death and cancels every session (which resolves any waiting ask).
    /// Modelled here: `write_frames` fed one frame over a sink that accepts the
    /// write but fails flush must terminate promptly; a cancellation wired to
    /// that termination (as `async_main`'s writer arm does) then unblocks a
    /// registered permission waiter, leaving zero pending entries and all
    /// permits free — not after the injected deadline.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_flush_failure_kills_writer_and_resolves_waiter() {
        let broker = Arc::new(PermissionBroker::new(4, LONG));
        // A live output channel; its frames are drained by `write_frames` into
        // the flush-failing sink, modelling the real writer over a broken pipe.
        let (wire_tx, wire_rx) = mpsc::channel(8);
        let (cancel_tx, mut cancel_rx) = watch::channel(false);

        // Supervisor: run the writer over the failing sink; when it returns
        // (flush error → connection-fatal), propagate cancellation exactly like
        // `async_main`'s writer-death arm.
        let writer = tokio::spawn(async move {
            wire::write_frames(wire_rx, FlushFailSink).await;
            let _ = cancel_tx.send(true);
        });

        let b = Arc::clone(&broker);
        let call = tool_call();
        let task = tokio::spawn(async move {
            b.request_permission(&wire_tx, 2, "ses_a", &call, &mut cancel_rx)
                .await
        });

        // The ask registers and enqueues its frame; the writer accepts the
        // write, fails the flush, returns, and the supervisor cancels. The
        // waiter must resolve via that cancellation, not the LONG deadline.
        let decision = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("flush failure must cancel the waiter, not wait the deadline")
            .unwrap();

        writer.await.unwrap();
        assert_eq!(decision, PermissionDecision::Cancelled);
        assert_eq!(
            broker.pending_count(),
            0,
            "writer death resolves the waiter and leaves no resident entry"
        );
        assert_eq!(
            broker.available_permits(),
            4,
            "writer death releases the held admission slot"
        );
    }

    // ── Enqueue backpressure is deadline- and cancel-governed ─────────────────

    /// A full-but-live output channel makes `send_checked` wait for capacity.
    /// The send phase races the single absolute deadline, so a stalled writer
    /// cannot hold the ask (and its global permit) past the advertised
    /// deadline: `request_permission` must return the timeout deny within the
    /// deadline and leave zero pending entries and zero held permits. Uses a
    /// SHORT deadline bounded by a longer outer timeout, so a regression that
    /// waits forever on capacity blows the outer bound. This is a distinct seam
    /// from the dropped-receiver test: here the receiver is alive but never
    /// drains.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_full_channel_send_is_bounded_by_the_deadline() {
        let broker = Arc::new(PermissionBroker::new(4, SHORT));
        // Capacity-1 channel, prefilled and never drained: the next send blocks
        // on capacity while the receiver stays alive (writer present, stalled).
        let (tx, _rx) = mpsc::channel(1);
        tx.send(wire::WireMsg::Notify(json!({ "fill": 1 })))
            .await
            .unwrap();
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);
        let call = tool_call();

        let decision = tokio::time::timeout(
            Duration::from_secs(2),
            broker.request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx),
        )
        .await
        .expect("a stalled writer must not hold the send past the deadline");

        assert_eq!(
            decision,
            PermissionDecision::Denied(PERMISSION_TIMEOUT_MSG),
            "a full-but-live channel resolves via the deadline, not the wire-closed path"
        );
        assert_eq!(
            broker.pending_count(),
            0,
            "a timed-out enqueue leaves no resident entry"
        );
        assert_eq!(
            broker.available_permits(),
            4,
            "a timed-out enqueue releases its admission slot"
        );
    }

    /// Cancellation must also resolve an ask stuck enqueueing on a stalled
    /// writer: with a full-but-live channel and a LONG deadline, a
    /// `session/cancel` returns `Cancelled` promptly rather than waiting the
    /// deadline out.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_cancel_resolves_a_blocked_enqueue() {
        let broker = Arc::new(PermissionBroker::new(4, LONG));
        let (tx, _rx) = mpsc::channel(1);
        tx.send(wire::WireMsg::Notify(json!({ "fill": 1 })))
            .await
            .unwrap();
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        let b = Arc::clone(&broker);
        let call = tool_call();
        let task = tokio::spawn(async move {
            b.request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx)
                .await
        });

        // Give the task time to admit + block on the full channel, then cancel.
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel_tx.send(true).unwrap();
        let decision = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("cancel must resolve a blocked enqueue promptly")
            .unwrap();
        assert_eq!(decision, PermissionDecision::Cancelled);
        assert_eq!(broker.pending_count(), 0);
        assert_eq!(broker.available_permits(), 4);
    }

    // ── Claim-before-wake ordering (mutation-sensitive) ───────────────────────

    /// The waiter must observe the correlation entry already *claimed* (removed
    /// from `pending`) at the instant it wakes with the delivered response —
    /// `deliver` removes before it sends. The wake observer fires synchronously
    /// inside the waiter's response arm, so it captures the exact `pending`
    /// state at the wake. A wake-before-claim mutant (send first, remove after)
    /// makes the observed state `false` and fails this assertion; the behavioral
    /// delivery tests cannot detect that mutant because the waiter reads the
    /// oneshot exactly once either way.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_waiter_observes_entry_claimed_before_wake() {
        let broker = Arc::new(PermissionBroker::new(4, LONG));
        let claimed_at_wake = Arc::new(Mutex::new(None::<bool>));
        let sink = Arc::clone(&claimed_at_wake);
        broker.set_wake_observer(Arc::new(move |claimed| {
            *sink.lock().unwrap() = Some(claimed);
        }));

        let (tx, mut rx) = mpsc::channel(8);
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);
        let b = Arc::clone(&broker);
        let call = tool_call();
        let task = tokio::spawn(async move {
            b.request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx)
                .await
        });

        let id = next_request_id(&mut rx).await;
        broker.deliver(&id, selected(ALLOW_OPTION_ID));
        assert_eq!(task.await.unwrap(), PermissionDecision::Allowed);
        assert_eq!(
            *claimed_at_wake.lock().unwrap(),
            Some(true),
            "entry must be claimed (removed) before the waiter is woken"
        );
    }

    // ── Cancellation while waiting ───────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_cancel_while_waiting_returns_cancelled_and_removes_state() {
        let broker = Arc::new(PermissionBroker::new(4, LONG));
        let (tx, mut rx) = mpsc::channel(8);
        let (cancel_tx, mut cancel_rx) = watch::channel(false);

        let b = Arc::clone(&broker);
        let call = tool_call();
        let task = tokio::spawn(async move {
            b.request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx)
                .await
        });

        let _id = next_request_id(&mut rx).await;
        assert_eq!(broker.pending_count(), 1);
        cancel_tx.send(true).unwrap();
        assert_eq!(task.await.unwrap(), PermissionDecision::Cancelled);
        assert_eq!(broker.pending_count(), 0);
        assert_eq!(broker.available_permits(), 4);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_precancelled_turn_never_sends_request() {
        let broker = Arc::new(PermissionBroker::new(4, LONG));
        let (tx, mut rx) = mpsc::channel(8);
        let (_cancel_tx, mut cancel_rx) = watch::channel(true); // already cancelled
        let call = tool_call();

        let decision = broker
            .request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx)
            .await;
        assert_eq!(decision, PermissionDecision::Cancelled);
        assert_eq!(broker.pending_count(), 0, "no entry inserted");
        assert_eq!(broker.available_permits(), 4);
        assert!(rx.try_recv().is_err(), "no request frame emitted");
    }

    // ── Abort-safe drop guard ─────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_abort_while_waiting_leaves_zero_pending_and_reusable_slot() {
        let broker = Arc::new(PermissionBroker::new(1, LONG));
        let (tx, mut rx) = mpsc::channel(8);
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);

        let b = Arc::clone(&broker);
        let call = tool_call();
        let task = tokio::spawn(async move {
            b.request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx)
                .await
        });

        // Registered + sent → then hard-abort the task (bypasses every normal
        // exit path). The lease's Drop must still run.
        let _id = next_request_id(&mut rx).await;
        assert_eq!(broker.pending_count(), 1);
        assert_eq!(broker.available_permits(), 0);
        task.abort();
        let _ = task.await;
        assert_eq!(
            broker.pending_count(),
            0,
            "drop guard removed the entry on abort"
        );
        assert_eq!(
            broker.available_permits(),
            1,
            "drop guard released the slot on abort"
        );
    }

    // ── Process-wide admission cap across multiple sessions ───────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_admission_cap_bounds_entries_across_sessions() {
        // Capacity 2, shared by all sessions. Three distinct sessions ask at
        // once; only two can register/send while the cap is saturated. The
        // third is admitted only after a slot frees. Frame ids arrive in
        // nondeterministic order across tasks, so the test never maps a task
        // handle to a specific id — it proves the bound structurally (frame
        // count + pending_count) and that every task ultimately resolves.
        let broker = Arc::new(PermissionBroker::new(2, LONG));
        let (tx, mut rx) = mpsc::channel(8);
        let (_c, cancel_rx) = watch::channel(false);

        let spawn_req = |session: &'static str| {
            let b = Arc::clone(&broker);
            let tx = tx.clone();
            let mut cancel = cancel_rx.clone();
            let call = tool_call();
            tokio::spawn(async move {
                b.request_permission(&tx, 2, session, &call, &mut cancel)
                    .await
            })
        };

        let tasks = [spawn_req("ses_a"), spawn_req("ses_b"), spawn_req("ses_c")];

        // Only two frames appear while the cap is 2; the third is blocked in
        // admission with no entry and no frame.
        let id1 = next_request_id(&mut rx).await;
        let id2 = next_request_id(&mut rx).await;
        assert_eq!(broker.pending_count(), 2);
        assert_eq!(broker.available_permits(), 0);
        assert!(
            tokio::time::timeout(SHORT, rx.recv()).await.is_err(),
            "third session must not send a request while the cap is saturated"
        );
        assert_eq!(
            broker.pending_count(),
            2,
            "cap holds: no third entry inserted"
        );

        // Free one slot → the third session is admitted and sends its frame.
        broker.deliver(&id1, selected(ALLOW_OPTION_ID));
        let id3 = next_request_id(&mut rx).await;
        assert_eq!(broker.pending_count(), 2, "still bounded after churn");

        // Resolve the two remaining live entries.
        broker.deliver(&id2, selected(ALLOW_OPTION_ID));
        broker.deliver(&id3, selected(ALLOW_OPTION_ID));

        // Every session resolved to Allowed — none stranded or timed out.
        for t in tasks {
            assert_eq!(t.await.unwrap(), PermissionDecision::Allowed);
        }
        assert_eq!(broker.pending_count(), 0);
        assert_eq!(broker.available_permits(), 2);
    }

    // ── Admission-phase cancel: fail-closed, zero entries ─────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_cancel_during_admission_inserts_no_entry() {
        // Saturate the single slot directly (test-module access to `sem`) so the
        // request under test blocks in the admission phase, before any insert.
        let broker = Arc::new(PermissionBroker::new(1, LONG));
        let held = Arc::clone(&broker.sem).acquire_owned().await.unwrap();
        assert_eq!(broker.available_permits(), 0);

        let (tx, mut rx) = mpsc::channel(8);
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        let b = Arc::clone(&broker);
        let call = tool_call();
        let task = tokio::spawn(async move {
            b.request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx)
                .await
        });

        // It cannot proceed past admission: no entry, no frame.
        assert!(tokio::time::timeout(SHORT, rx.recv()).await.is_err());
        assert_eq!(broker.pending_count(), 0, "blocked before insert");

        cancel_tx.send(true).unwrap();
        assert_eq!(task.await.unwrap(), PermissionDecision::Cancelled);
        assert_eq!(
            broker.pending_count(),
            0,
            "cancel during admission inserts nothing"
        );
        drop(held);
        assert_eq!(broker.available_permits(), 1);
    }

    // ── Admission-phase deadline: fail-closed deny, zero entries ──────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_deadline_during_admission_denies_with_no_entry() {
        let broker = Arc::new(PermissionBroker::new(1, SHORT));
        let held = Arc::clone(&broker.sem).acquire_owned().await.unwrap();

        let (tx, mut rx) = mpsc::channel(8);
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);
        let call = tool_call();

        // Slot never frees within the deadline → admission times out.
        let decision = broker
            .request_permission(&tx, 2, "ses_a", &call, &mut cancel_rx)
            .await;
        assert_eq!(decision, PermissionDecision::Denied(PERMISSION_TIMEOUT_MSG));
        assert_eq!(
            broker.pending_count(),
            0,
            "no entry inserted on admission timeout"
        );
        assert!(rx.try_recv().is_err(), "no request frame emitted");
        drop(held);
    }
}
