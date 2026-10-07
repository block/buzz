//! Lifecycle tests for [`super`]'s CLOSED recovery and subscription bookkeeping.
//!
//! Split out of `native_relay_client.rs` to keep that file under the desktop
//! file-size ratchet. Same `#[path]` sibling-module convention as
//! `archive/sync.rs` and its `sync_tests.rs`.

use super::*;
use futures_util::{SinkExt, StreamExt};
use nostr::EventBuilder;
use tokio_tungstenite::tungstenite::protocol::Message;

/// The subscription id every test below drives.
const PROBE_ID: &str = "archive:probe";

/// Minimal relay that completes the NIP-42 handshake, records every REQ,
/// and sends a CLOSED only when the test asks it to.
///
/// A real socket rather than a fake `NostrWsConnection`, because the bug
/// this covers lives in the lifecycle between frames — the loop's only
/// reconcile triggers — and a fake that hands the loop a `Closed` value
/// cannot show that a REQ went back out over the wire afterwards. Same
/// `accept_async` stub shape as `native_websocket.rs`'s live-TCP tests.
///
/// CLOSED is test-driven rather than a scripted reply to the first REQ so
/// the test can wait for the session to go quiet first. `set_subscriptions`
/// queues a wake that may still be pending when an immediate CLOSED lands,
/// and that wake reopens the subscription on its own — which made the first
/// version of this test pass against the unfixed code.
///
/// `frames` reports REQ and CLOSE in wire order, not REQ alone: the
/// lifecycle tests below assert that a CLOSE was sent before the REQ that
/// follows it, which a REQ-only channel cannot express.
async fn stub_relay() -> (String, mpsc::Receiver<Frame>, mpsc::Sender<StubCommand>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stub relay");
    let address = listener.local_addr().expect("stub relay address");
    let (req_tx, req_rx) = mpsc::channel(16);
    let (closed_tx, mut closed_rx) = mpsc::channel::<StubCommand>(4);

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("websocket handshake");

        socket
            .send(Message::Text(r#"["AUTH","stub-challenge"]"#.into()))
            .await
            .expect("send challenge");

        loop {
            tokio::select! {
                incoming = socket.next() => {
                    let Some(Ok(Message::Text(text))) = incoming else { return };
                    let Ok(frame) = serde_json::from_str::<serde_json::Value>(&text) else {
                        continue;
                    };
                    match frame[0].as_str() {
                        Some("AUTH") => {
                            let id = frame[1]["id"].as_str().unwrap_or_default();
                            socket
                                .send(Message::Text(
                                    serde_json::json!(["OK", id, true, ""]).to_string().into(),
                                ))
                                .await
                                .expect("send auth ok");
                        }
                        Some("REQ") => {
                            let id = frame[1].as_str().unwrap_or_default().to_string();
                            if req_tx.send(Frame::Req(id)).await.is_err() {
                                return;
                            }
                        }
                        Some("CLOSE") => {
                            let id = frame[1].as_str().unwrap_or_default().to_string();
                            if req_tx.send(Frame::Close(id)).await.is_err() {
                                return;
                            }
                        }
                        _ => {}
                    }
                }
                Some(command) = closed_rx.recv() => {
                    let frame = match command {
                        StubCommand::Closed(id, message) => {
                            serde_json::json!(["CLOSED", id, message])
                        }
                        StubCommand::Eose(id) => serde_json::json!(["EOSE", id]),
                        StubCommand::Event(id, event) => {
                            serde_json::json!(["EVENT", id, event])
                        }
                    };
                    socket
                        .send(Message::Text(frame.to_string().into()))
                        .await
                        .expect("send stub frame");
                }
            }
        }
    });

    (format!("ws://{address}"), req_rx, closed_tx)
}

/// A client→relay frame the stub observed, in wire order.
#[derive(Debug, PartialEq, Eq)]
enum Frame {
    Req(String),
    Close(String),
}

/// A relay→client frame the test asks the stub to emit.
enum StubCommand {
    Closed(String, String),
    Eose(String),
    Event(String, serde_json::Value),
}

fn probe_subscription() -> Subscription {
    Subscription {
        id: PROBE_ID.to_string(),
        filter: serde_json::json!({ "kinds": [1], "limit": 0 }),
    }
}

async fn next_frame(frames: &mut mpsc::Receiver<Frame>, label: &str) -> Frame {
    tokio::time::timeout(Duration::from_secs(10), frames.recv())
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {label}"))
        .unwrap_or_else(|| panic!("stub relay closed before {label}"))
}

/// Waits for the next REQ, tolerating the CLOSE frames a reconcile sends
/// first. Asserting on `Frame::Req` directly would couple every test to
/// whether a particular reconcile also had cleanup to do.
async fn next_req(frames: &mut mpsc::Receiver<Frame>, label: &str) -> String {
    loop {
        if let Frame::Req(id) = next_frame(frames, label).await {
            return id;
        }
    }
}

/// Waits out the wake `set_subscriptions` queued, so a CLOSED sent after
/// this cannot be reopened by anything but the CLOSED path itself.
///
/// A pending wake is harmless while the subscription is still open — that
/// reconcile is a no-op — so draining it before the CLOSED is what makes
/// the assertion below attributable.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(500)).await;
}

/// C's acceptance edge: a finite request shares the authenticated real socket
/// with a persistent subscription, completes on wire EOSE, and does not steal
/// later persistent delivery. A fake connection cannot establish any of those
/// transport/lifetime properties.
#[tokio::test]
async fn finite_fetch_multiplexes_with_persistent_delivery_on_a_real_websocket() {
    let (relay_url, mut frames, commands) = stub_relay().await;
    let (session, mut events) = start(relay_url, Keys::generate(), None).await;
    session.set_subscriptions(vec![probe_subscription()]).await;
    assert_eq!(next_req(&mut frames, "the persistent REQ").await, PROBE_ID);

    let fetch = {
        let session = Arc::clone(&session);
        tokio::spawn(async move {
            session
                .fetch_events(
                    serde_json::json!({ "kinds": [buzz_core_pkg::kind::KIND_PERSONA], "limit": 500 }),
                    Duration::from_secs(10),
                )
                .await
        })
    };
    let request_id = next_req(&mut frames, "the finite fetch REQ").await;
    assert_ne!(request_id, PROBE_ID);

    let relay_keys = Keys::generate();
    let mut forged = EventBuilder::text_note("forged catalog page event")
        .sign_with_keys(&relay_keys)
        .unwrap();
    forged.content = "tampered after signing".into();
    commands
        .send(StubCommand::Event(
            request_id.clone(),
            serde_json::to_value(forged).unwrap(),
        ))
        .await
        .unwrap();
    let fetched = EventBuilder::text_note("catalog page event")
        .sign_with_keys(&relay_keys)
        .unwrap();
    commands
        .send(StubCommand::Event(
            request_id.clone(),
            serde_json::to_value(&fetched).unwrap(),
        ))
        .await
        .unwrap();
    commands
        .send(StubCommand::Eose(request_id.clone()))
        .await
        .unwrap();

    assert_eq!(fetch.await.unwrap().unwrap(), vec![fetched]);
    assert_eq!(
        next_frame(&mut frames, "finite fetch CLOSE").await,
        Frame::Close(request_id)
    );

    let persistent = EventBuilder::text_note("persistent event after fetch")
        .sign_with_keys(&relay_keys)
        .unwrap();
    commands
        .send(StubCommand::Event(
            PROBE_ID.into(),
            serde_json::to_value(&persistent).unwrap(),
        ))
        .await
        .unwrap();
    let delivered = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivered.subscription_id, PROBE_ID);
    assert_eq!(*delivered.event, persistent);
    session.shutdown();
}

async fn run_persistent_burst(drain_concurrently: bool) {
    const BURST: usize = 1_200;

    let (relay_url, mut frames, commands) = stub_relay().await;
    let (session, mut events) = start(relay_url, Keys::generate(), None).await;
    session.set_subscriptions(vec![probe_subscription()]).await;
    assert_eq!(next_req(&mut frames, "the burst REQ").await, PROBE_ID);

    let relay_keys = Keys::generate();
    let event = EventBuilder::text_note("persistent burst event")
        .sign_with_keys(&relay_keys)
        .unwrap();
    let send_burst = tokio::spawn({
        let commands = commands.clone();
        let event = serde_json::to_value(&event).unwrap();
        async move {
            for _ in 0..BURST {
                commands
                    .send(StubCommand::Event(PROBE_ID.into(), event.clone()))
                    .await
                    .unwrap();
            }
        }
    });

    if !drain_concurrently {
        // Let the bounded archive channel fill before draining. The socket loop
        // must wait here rather than evicting live-only events.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for _ in 0..BURST {
        tokio::time::timeout(Duration::from_secs(60), events.recv())
            .await
            .expect("timed out draining persistent burst")
            .expect("archive receiver closed during persistent burst");
    }
    send_burst.await.unwrap();

    let after = EventBuilder::text_note("persistent event after burst")
        .sign_with_keys(&relay_keys)
        .unwrap();
    commands
        .send(StubCommand::Event(
            PROBE_ID.into(),
            serde_json::to_value(&after).unwrap(),
        ))
        .await
        .unwrap();
    let delivered = tokio::time::timeout(Duration::from_secs(60), events.recv())
        .await
        .expect("timed out after persistent burst")
        .expect("archive receiver closed after persistent burst");
    assert_eq!(*delivered.event, after);
    session.shutdown();
}

/// Persistent archive subscriptions use `limit: 0`, so an event lost during a
/// slow-consumer burst cannot be replayed. Both a fast control and a receiver
/// that starts late must therefore get the whole burst and remain live after it.
#[tokio::test]
async fn persistent_delivery_applies_backpressure_without_losing_a_burst() {
    run_persistent_burst(true).await;
    run_persistent_burst(false).await;
}

/// The blocker: a CLOSED with the desired set never changing again must
/// still reopen the subscription.
///
/// Before the fix the loop removed the id from `open` and waited on a wake
/// that only `set_subscriptions` can produce, so a stable desired set left
/// the subscription dead for the life of the socket — silent permanent
/// loss for ephemeral kind 24200.
#[tokio::test]
async fn a_closed_subscription_reopens_without_a_desired_set_change() {
    let (relay_url, mut frames, closed) = stub_relay().await;
    let (session, _events) = start(relay_url, Keys::generate(), None).await;

    session.set_subscriptions(vec![probe_subscription()]).await;
    assert_eq!(next_req(&mut frames, "the initial REQ").await, PROBE_ID);
    settle().await;

    // Retryable class, sent once: the reopen is answered normally, so a
    // failure here means "never retried" rather than "retried into another
    // rejection".
    closed
        .send(StubCommand::Closed(
            PROBE_ID.into(),
            "error: temporary".into(),
        ))
        .await
        .expect("stub relay accepts the closed command");

    // No `set_subscriptions` between the two REQs: the reopen must come
    // from the CLOSED itself, which is exactly the edge that was missing.
    assert_eq!(next_req(&mut frames, "the reopened REQ").await, PROBE_ID);

    session.shutdown();
}

/// A relay that rejects on policy must not be re-asked in a tight loop.
#[tokio::test]
async fn a_terminal_closed_is_not_retried_on_the_same_socket() {
    let (relay_url, mut frames, closed) = stub_relay().await;
    let (session, _events) = start(relay_url, Keys::generate(), None).await;

    session.set_subscriptions(vec![probe_subscription()]).await;
    assert_eq!(next_req(&mut frames, "the initial REQ").await, PROBE_ID);
    settle().await;

    closed
        .send(StubCommand::Closed(
            PROBE_ID.into(),
            "restricted: not authorized".into(),
        ))
        .await
        .expect("stub relay accepts the closed command");

    // Long enough that a retryable class (1s base) would have reopened
    // several times, so this asserts suppression rather than just slowness.
    let retried = tokio::time::timeout(Duration::from_secs(5), frames.recv()).await;
    assert!(
        retried.is_err(),
        "a terminal CLOSED must not be retried on this socket, got {retried:?}"
    );

    session.shutdown();
}

/// M18: a subscription deleted and recreated must get a fresh REQ, even
/// though its terminal latch says never to retry.
///
/// The latch is scoped to the subscription that earned it. Recreating the
/// id is a new subscription that happens to share a name — `archive::sync`
/// derives the id from scope and kinds, so a delete/recreate of the same
/// saved subscription produces a byte-identical id and would otherwise
/// inherit a permanent suppression for the life of the socket.
#[tokio::test]
async fn a_recreated_subscription_does_not_inherit_a_terminal_latch() {
    let (relay_url, mut frames, closed) = stub_relay().await;
    let (session, _events) = start(relay_url, Keys::generate(), None).await;

    session.set_subscriptions(vec![probe_subscription()]).await;
    assert_eq!(next_req(&mut frames, "the initial REQ").await, PROBE_ID);
    settle().await;

    closed
        .send(StubCommand::Closed(
            PROBE_ID.into(),
            "restricted: not authorized".into(),
        ))
        .await
        .expect("stub relay accepts the closed command");
    settle().await;

    // Delete, then recreate — each observed as its own reconcile.
    session.set_subscriptions(vec![]).await;
    settle().await;
    session.set_subscriptions(vec![probe_subscription()]).await;

    assert_eq!(
        next_req(&mut frames, "the REQ for the recreated subscription").await,
        PROBE_ID,
    );

    session.shutdown();
}

/// M19: the same schedule, with both writes landing before the loop
/// consumes its single wake.
///
/// This is the mutant that discriminates the mechanism. The wake channel
/// has capacity 1 and `set_subscriptions` only ever queues "reconcile
/// pending", so the delete and the recreate collapse into ONE observed
/// reconcile whose desired set already contains the id again. A prune that
/// reads only the current desired set never sees the id absent and leaves
/// the latch in place — passing the test above while failing this one.
/// The departure is therefore recorded at write time, where it is visible.
///
/// No `settle()` between the two writes: that gap is the whole point, and
/// adding one would silently convert this into a duplicate of M18.
#[tokio::test]
async fn a_recreated_subscription_is_not_suppressed_when_the_writes_coalesce() {
    let (relay_url, mut frames, closed) = stub_relay().await;
    let (session, _events) = start(relay_url, Keys::generate(), None).await;

    session.set_subscriptions(vec![probe_subscription()]).await;
    assert_eq!(next_req(&mut frames, "the initial REQ").await, PROBE_ID);
    settle().await;

    closed
        .send(StubCommand::Closed(
            PROBE_ID.into(),
            "restricted: not authorized".into(),
        ))
        .await
        .expect("stub relay accepts the closed command");
    settle().await;

    session.set_subscriptions(vec![]).await;
    session.set_subscriptions(vec![probe_subscription()]).await;

    assert_eq!(
        next_req(&mut frames, "the REQ for the recreated subscription").await,
        PROBE_ID,
    );

    session.shutdown();
}

/// M20: pruning must be scoped to departures, not run every pass.
///
/// A reconcile triggered while the id is still desired must leave its
/// pending backoff alone. Clearing wholesale would collapse the CLOSED
/// backoff — every unrelated subscription change would re-ask a relay that
/// just rejected us, at the speed of the event loop.
#[tokio::test]
async fn a_reconcile_preserves_the_backoff_of_a_still_desired_subscription() {
    let (relay_url, mut frames, closed) = stub_relay().await;
    let (session, _events) = start(relay_url, Keys::generate(), None).await;

    session.set_subscriptions(vec![probe_subscription()]).await;
    assert_eq!(next_req(&mut frames, "the initial REQ").await, PROBE_ID);
    settle().await;

    // Rate-limited: a long, unambiguously pending backoff, so a reopen
    // inside the window is the prune and not the timer.
    closed
        .send(StubCommand::Closed(
            PROBE_ID.into(),
            "rate-limited: slow down; retry in 30s".into(),
        ))
        .await
        .expect("stub relay accepts the closed command");
    settle().await;

    // A change that adds an unrelated subscription. The probe never leaves
    // the desired set, so its backoff must survive this reconcile.
    session
        .set_subscriptions(vec![
            probe_subscription(),
            Subscription {
                id: "archive:other".to_string(),
                filter: serde_json::json!({ "kinds": [7], "limit": 0 }),
            },
        ])
        .await;

    assert_eq!(
        next_req(&mut frames, "the REQ for the newly added subscription").await,
        "archive:other",
    );
    let reopened = tokio::time::timeout(Duration::from_secs(3), frames.recv()).await;
    assert!(
        reopened.is_err(),
        "a still-desired subscription must keep its pending backoff across a \
         reconcile, got {reopened:?}"
    );

    crate::relay_admission::reset_rate_limit_gate();
    session.shutdown();
}

/// M21: a CLOSED that arrives after we stopped running the subscription is
/// stale and must mint nothing.
///
/// Our CLOSE races the relay's in-flight frames — the EVENT arm already
/// guards this. Without the same guard on CLOSED, the frame recreates the
/// retry entry the drain just removed, and nothing can evict it: the id is
/// gone from the desired set, so no future departure records it again.
#[tokio::test]
async fn a_closed_arriving_after_removal_does_not_mint_retry_state() {
    let (relay_url, mut frames, closed) = stub_relay().await;
    let (session, _events) = start(relay_url, Keys::generate(), None).await;

    session.set_subscriptions(vec![probe_subscription()]).await;
    assert_eq!(next_req(&mut frames, "the initial REQ").await, PROBE_ID);
    settle().await;

    // Delete first, and wait for our CLOSE to reach the wire: that ordering
    // is what makes the CLOSED below arrive after the drain rather than
    // before it, which is the schedule M18 and M19 do not cover.
    session.set_subscriptions(vec![]).await;
    assert_eq!(
        next_frame(&mut frames, "the CLOSE for the deleted subscription").await,
        Frame::Close(PROBE_ID.to_string()),
    );

    closed
        .send(StubCommand::Closed(
            PROBE_ID.into(),
            "restricted: not authorized".into(),
        ))
        .await
        .expect("stub relay accepts the closed command");
    settle().await;

    session.set_subscriptions(vec![probe_subscription()]).await;

    assert_eq!(
        next_req(&mut frames, "the REQ for the recreated subscription").await,
        PROBE_ID,
    );

    session.shutdown();
}

/// M22: a stale *terminal* CLOSED landing after the id was recreated must
/// not blackhole the live subscription.
///
/// This one survives every defense above. The CLOSED is legitimately
/// attributed — the id is open again, so the M21 guard passes it — and
/// terminal means no `due_at`, so the timer arm is disabled and no wake is
/// pending. `open` loses the id while the relay keeps delivering, and the
/// EVENT arm drops every frame in silence.
///
/// EOSE is the recovery edge because it is the only ordered fence
/// available: frames on one socket are totally ordered, so the previous
/// generation's CLOSED necessarily precedes the new generation's EOSE.
#[tokio::test]
async fn a_stale_terminal_closed_does_not_blackhole_a_recreated_subscription() {
    let (relay_url, mut frames, closed) = stub_relay().await;
    let (session, mut events) = start(relay_url, Keys::generate(), None).await;

    session.set_subscriptions(vec![probe_subscription()]).await;
    assert_eq!(next_req(&mut frames, "the initial REQ").await, PROBE_ID);
    settle().await;

    // Delete and recreate, so the id is open again under a new generation.
    session.set_subscriptions(vec![]).await;
    assert_eq!(
        next_frame(&mut frames, "the CLOSE for the deleted subscription").await,
        Frame::Close(PROBE_ID.to_string()),
    );
    session.set_subscriptions(vec![probe_subscription()]).await;
    assert_eq!(
        next_req(&mut frames, "the REQ for the recreated subscription").await,
        PROBE_ID,
    );
    settle().await;

    // The old generation's terminal CLOSED, delayed past the new REQ.
    closed
        .send(StubCommand::Closed(
            PROBE_ID.into(),
            "restricted: not authorized".into(),
        ))
        .await
        .expect("stub relay accepts the closed command");
    // The new generation's EOSE, which the wire orders after it.
    closed
        .send(StubCommand::Eose(PROBE_ID.into()))
        .await
        .expect("stub relay accepts the eose command");

    // The EOSE found the id closed, so it must drive a reconcile that
    // reopens it. Nothing else can: terminal schedules no timer, and the
    // desired set is stable.
    assert_eq!(
        next_req(&mut frames, "the REQ healing the open-map mismatch").await,
        PROBE_ID,
    );

    // And the heal converges rather than storming: the replacement EOSE
    // finds the id open, so it wakes nothing.
    closed
        .send(StubCommand::Eose(PROBE_ID.into()))
        .await
        .expect("stub relay accepts the second eose command");
    let extra = tokio::time::timeout(Duration::from_secs(3), frames.recv()).await;
    assert!(
        extra.is_err(),
        "an EOSE for an already-open subscription must not re-reconcile, got {extra:?}"
    );

    // The point of the heal: events flow again.
    let event = EventBuilder::text_note("post-heal")
        .sign_with_keys(&Keys::generate())
        .expect("sign event");
    let event_id = event.id.to_hex();
    closed
        .send(StubCommand::Event(
            PROBE_ID.into(),
            serde_json::to_value(&event).expect("serialize event"),
        ))
        .await
        .expect("stub relay accepts the event command");

    let delivered = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("timed out waiting for an event after the heal")
        .expect("session channel closed");
    assert_eq!(
        delivered.event.id.to_hex(),
        event_id,
        "events must flow again once the open map is healed"
    );

    session.shutdown();
}

/// M23: reusing an id for a changed filter must be *detected*.
///
/// This test pins detection and nothing else. Post-violation behavior —
/// whether the subscription reopens, what happens to its retry state, what
/// the relay is sent — is unspecified by design, because the wire carries
/// only the id and an in-flight CLOSED from the old filter is
/// indistinguishable from one caused by the new one. Asserting any of that
/// would turn an unsupported input into a supported one.
///
/// It exists because the `(id, filter)` departure diff is otherwise
/// unpinned: on every supported path it is byte-equivalent to an id-only
/// diff, so a refactor could revert it, pass every other test here, and
/// silently remove the one signal that tells C and D they broke the
/// contract.
#[test]
fn a_filter_change_under_a_reused_id_is_reported_as_a_contract_violation() {
    let mut state = SessionState::default();

    assert!(
        state.replace_desired(vec![probe_subscription()]).is_empty(),
        "a first desired set violates nothing"
    );
    assert!(
        state.replace_desired(vec![probe_subscription()]).is_empty(),
        "an unchanged subscription is not a filter change"
    );

    let violations = state.replace_desired(vec![Subscription {
        id: PROBE_ID.to_string(),
        filter: serde_json::json!({ "kinds": [7], "limit": 0 }),
    }]);

    assert_eq!(
        violations,
        vec![PROBE_ID.to_string()],
        "a filter changed under a reused id must be reported"
    );
}

#[test]
fn closed_messages_classify_like_the_renderer_policy() {
    assert_eq!(
        classify_closed("rate-limited: quota exceeded; retry in 4s"),
        ClosedClass::RateLimited
    );
    assert_eq!(
        classify_closed("restricted: not authorized"),
        ClosedClass::Terminal
    );
    assert_eq!(
        classify_closed("error: too many subscriptions"),
        ClosedClass::Terminal
    );
    // Transient AUTH race, not a permanent rejection — the one prefix that
    // looks terminal and deliberately is not.
    assert_eq!(
        classify_closed("auth-required: we can't serve unauthenticated"),
        ClosedClass::Retryable
    );
    assert_eq!(classify_closed(""), ClosedClass::Retryable);
    // Case and padding come from the relay, not from us.
    assert_eq!(
        classify_closed("  RESTRICTED: nope  "),
        ClosedClass::Terminal
    );
}

#[test]
fn retry_delay_grows_and_stops_at_the_ceiling() {
    let mut retry = ClosedRetry::default();
    assert_eq!(retry.backoff(), CLOSED_RETRY_BASE_DELAY);

    retry.schedule("error: temporary");
    assert_eq!(retry.backoff(), CLOSED_RETRY_BASE_DELAY * 2);

    for _ in 0..40 {
        retry.schedule("error: temporary");
    }
    assert_eq!(
        retry.backoff(),
        CLOSED_RETRY_MAX_DELAY,
        "backoff must saturate at the ceiling rather than wrapping"
    );
}

#[test]
fn a_rate_limited_closed_waits_at_least_the_relay_hint() {
    let mut retry = ClosedRetry::default();
    retry.schedule("rate-limited: quota exceeded; retry in 12s");

    let due = retry.due_at.expect("rate-limited must schedule a reopen");
    // The hint dominates the 1s first backoff, so this asserts the hint was
    // honored rather than that anything at all was scheduled.
    assert!(
        due >= Instant::now() + Duration::from_secs(11),
        "a 12s hint must not be undercut by the base backoff"
    );
    crate::relay_admission::reset_rate_limit_gate();
}

#[test]
fn a_hintless_rate_limited_closed_uses_the_shared_default() {
    let mut retry = ClosedRetry::default();
    retry.schedule("rate-limited: quota exceeded");

    let due = retry.due_at.expect("rate-limited must schedule a reopen");
    assert!(
        due >= Instant::now() + CLOSED_RATE_LIMIT_DEFAULT - Duration::from_secs(1),
        "a hintless rate-limit must fall back to the shared default window"
    );
    crate::relay_admission::reset_rate_limit_gate();
}

#[test]
fn retry_hints_parse_the_relays_canonical_format() {
    assert_eq!(
        parse_retry_in_seconds("rate-limited: quota exceeded; retry in 4s"),
        Some(4)
    );
    assert_eq!(parse_retry_in_seconds("rate-limited: quota exceeded"), None);
    assert_eq!(parse_retry_in_seconds("retry in s"), None);
}

// ── Scope fencing at the client boundary ─────────────────────────────────────
//
// `ensure_session` is destructive on entry: a different scope's socket is shut
// down before the new one is installed. The archive lifecycle earns that right
// with `ArchiveOwnership`; the persona catalog and unread catch-up hold no such
// proof and reach the client through `session` instead.
//
// These tests drive `ensure_session` directly rather than `archive_session`,
// because `ArchiveOwnership` is un-constructible outside `archive::sync` — the
// compiler already enforces that half. `archive_session` delegates to
// `ensure_session` with no other effect on the slot, so this stages the exact
// state a live archive leaves behind.
//
// The relay URLs never accept a connection. Nothing here waits on a socket:
// the session task is spawned, its connect fails, and it backs off — while the
// slot bookkeeping and cancellation these tests assert on are synchronous.

/// A scope's relay URL. Distinct ports, on a closed loopback address, so the
/// two scopes are unequal and neither can connect.
fn scope_url(port: u16) -> String {
    format!("ws://127.0.0.1:{port}")
}

async fn installed_session(client: &NativeRelayClient) -> Option<Arc<RelaySession>> {
    client
        .current
        .lock()
        .await
        .as_ref()
        .map(|managed| Arc::clone(&managed.session))
}

/// The required regression: a finite request that resumes after the scope
/// switched must not disturb the new scope's live session.
///
/// Staged in the order the bug needs — archive A installed, scope switches and
/// archive B installs, and only then does A's delayed fetch acquire. Against
/// the unfenced `session` (a straight `ensure_session` call) A's late arrival
/// shut B's socket down and installed its own, leaving B's archive attached to
/// a cancelled session: no events, no error, until the next lifecycle edge.
#[tokio::test]
async fn a_stale_finite_request_cannot_displace_the_new_scopes_session() {
    let client = NativeRelayClient::default();
    let scope_a = (scope_url(9), Keys::generate());
    let scope_b = (scope_url(10), Keys::generate());

    let archive_a = client
        .ensure_session(scope_a.0.clone(), scope_a.1.clone())
        .await;
    let archive_b = client
        .ensure_session(scope_b.0.clone(), scope_b.1.clone())
        .await;
    assert!(
        archive_a.cancel.is_cancelled(),
        "the archive lifecycle must still replace its own scope's session"
    );

    // Scope A's in-flight catalog/catch-up command, resuming late.
    let stale = client.session(scope_a.0.clone(), scope_a.1.clone()).await;

    assert!(
        !archive_b.cancel.is_cancelled(),
        "a stale finite request cancelled the live scope's session; its archive \
         is now attached to a dead socket and will sit silent until the next \
         lifecycle edge"
    );
    let installed = installed_session(&client)
        .await
        .expect("the slot must still hold a session");
    assert!(
        Arc::ptr_eq(&installed, &archive_b),
        "a stale finite request replaced the installed session, so the next \
         same-scope caller shares the wrong socket"
    );
    assert!(
        !Arc::ptr_eq(&stale.session, &archive_b),
        "the stale request must run on its own session, not the live scope's"
    );

    // Its own session is the lease's to end, and it must actually end: an
    // un-cancelled private session leaks a reconnecting socket per request.
    let private = stale.handle();
    drop(stale);
    assert!(
        private.cancel.is_cancelled(),
        "dropping a private lease must shut its session down"
    );
}

/// The sharing half, and the mutant that matters: making every lease private
/// would satisfy the test above while quietly undoing the one-socket design and
/// letting a finite request's drop cancel the archive's session.
#[tokio::test]
async fn a_same_scope_lease_shares_the_installed_session_and_never_ends_it() {
    let client = NativeRelayClient::default();
    let (relay_url, keys) = (scope_url(11), Keys::generate());

    let archive = client.ensure_session(relay_url.clone(), keys.clone()).await;
    let lease = client.session(relay_url.clone(), keys.clone()).await;
    assert!(
        Arc::ptr_eq(&lease.session, &archive),
        "a same-scope finite request must multiplex over the installed socket \
         rather than opening a second one"
    );

    drop(lease);
    assert!(
        !archive.cancel.is_cancelled(),
        "dropping a shared lease cancelled the archive's session"
    );
    assert!(
        installed_session(&client)
            .await
            .is_some_and(|installed| Arc::ptr_eq(&installed, &archive)),
        "the shared session must stay installed after a lease is dropped"
    );
}

/// A lease taken before any archive start installs, so the archive start that
/// follows reuses that socket instead of opening a second one. This is the
/// common boot order: the catalog fetch runs before archive sync.
#[tokio::test]
async fn the_first_lease_installs_a_session_the_archive_then_reuses() {
    let client = NativeRelayClient::default();
    let (relay_url, keys) = (scope_url(12), Keys::generate());

    let lease = client.session(relay_url.clone(), keys.clone()).await;
    let leased = lease.handle();
    drop(lease);
    assert!(
        !leased.cancel.is_cancelled(),
        "the first lease owns the slot, so dropping it must not cancel the \
         session the archive is about to reuse"
    );

    let archive = client.ensure_session(relay_url, keys).await;
    assert!(
        Arc::ptr_eq(&archive, &leased),
        "the archive start must reuse the installed session rather than \
         replacing an identically scoped one"
    );
}
