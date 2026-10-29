//! Error CLOSED handling: per-subscription backoff instead of an immediate
//! re-REQ, and the replay floor a CLOSED subscription keeps.
//!
//! The relay sends `error: database error` when a REQ's history read fails,
//! and EOSE never follows. The retry must back off, and its `since` must still
//! cover the history that failed to arrive.
use super::resume_tests::{
    deliver_eose, first_replay_req, lose_connection, mock_relay, next_req, open_replay, LossArm,
    MockCtl,
};
use super::tests::{seed_test_subscription, test_channel_filter, test_ws_pair};
use super::*;

const DB_ERROR: &str = "error: database error";

/// Feed `["CLOSED", sub_id, message]` through the production frame handler.
async fn deliver_closed(client: &mut WsStream, state: &mut BgState, sub_id: &str, message: &str) {
    let (event_tx, _event_rx) = mpsc::channel(1);
    let (control_tx, _control_rx) = mpsc::channel(1);
    let alive = handle_ws_message(
        Message::Text(json!(["CLOSED", sub_id, message]).to_string().into()),
        client,
        &event_tx,
        &control_tx,
        state,
        &Keys::generate(),
        "ws://127.0.0.1:1",
        None,
    )
    .await;
    assert!(alive, "an error CLOSED must keep the socket");
}

/// Assert `at` is `delay` from `before`, within the ±20% jitter band plus a
/// little test runtime.
fn assert_due_after(at: tokio::time::Instant, before: tokio::time::Instant, delay: Duration) {
    let waited = at.saturating_duration_since(before);
    assert!(
        waited >= delay.mul_f64(0.8) && waited <= delay.mul_f64(1.2) + Duration::from_millis(500),
        "retry due after {waited:?}, expected {delay:?} ±20%"
    );
}

#[test]
fn closed_retry_delay_doubles_to_the_cap() {
    let secs = |failures| closed_retry_delay(failures).as_secs();
    let table = [
        (0, 2),
        (1, 2),
        (2, 4),
        (3, 8),
        (4, 16),
        (5, 32),
        (6, 64),
        (7, 120),
        (8, 120),
        (u32::MAX, 120),
    ];
    for (failures, expected) in table {
        assert_eq!(secs(failures), expected, "failures={failures}");
    }
}

/// A CLOSED channel is parked, not re-REQed at once. Each further CLOSED with
/// no EOSE in between doubles the delay; an EOSE resets it.
#[tokio::test]
async fn error_closed_parks_the_channel_with_growing_backoff_until_eose() {
    let (mut client, mut server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel);
    let sub = channel_sub_id(channel);

    for delay_secs in [2, 4, 8] {
        let before = tokio::time::Instant::now();
        deliver_closed(&mut client, &mut state, &sub, DB_ERROR).await;
        assert_due_after(
            state.rate_limited_pending[&channel],
            before,
            Duration::from_secs(delay_secs),
        );
        assert!(
            state.active_subscriptions.contains_key(&channel),
            "the channel stays subscribed so a reconnect restores it"
        );
    }
    assert!(
        timeout(Duration::from_millis(200), server.next())
            .await
            .is_err(),
        "an error CLOSED must not re-send the REQ immediately"
    );

    deliver_eose(&mut client, &mut state, &sub).await;
    assert!(!state.closed_failures.contains_key(&sub));
    let before = tokio::time::Instant::now();
    deliver_closed(&mut client, &mut state, &sub, DB_ERROR).await;
    assert_due_after(
        state.rate_limited_pending[&channel],
        before,
        Duration::from_secs(2),
    );
}

#[tokio::test]
async fn error_closed_parks_membership_and_observer_control_too() {
    let (mut client, mut server) = test_ws_pair().await;
    let mut state = BgState::new();
    state.membership_sub_active = true;
    state.observer_control_sub_active = true;

    for _ in 0..2 {
        deliver_closed(&mut client, &mut state, MEMBERSHIP_NOTIF_SUB_ID, DB_ERROR).await;
        deliver_closed(&mut client, &mut state, OBSERVER_CONTROL_SUB_ID, DB_ERROR).await;
    }
    let before = tokio::time::Instant::now();
    deliver_closed(&mut client, &mut state, MEMBERSHIP_NOTIF_SUB_ID, DB_ERROR).await;
    deliver_closed(&mut client, &mut state, OBSERVER_CONTROL_SUB_ID, DB_ERROR).await;
    // Third consecutive failure of each: 8s.
    assert_due_after(
        state.membership_resub_at.unwrap(),
        before,
        Duration::from_secs(8),
    );
    assert_due_after(
        state.observer_resub_at.unwrap(),
        before,
        Duration::from_secs(8),
    );
    assert!(
        timeout(Duration::from_millis(200), server.next())
            .await
            .is_err(),
        "an error CLOSED must not re-send the REQ immediately"
    );
}

/// CLOSEDs for subscriptions this task doesn't own (an unsubscribed channel, a
/// relay-chosen ID) park nothing and leave no failure record.
#[tokio::test]
async fn error_closed_for_an_unowned_subscription_is_ignored() {
    let (mut client, _server) = test_ws_pair().await;
    let mut state = BgState::new();
    let left = Uuid::new_v4();
    seed_test_subscription(&mut state, left);
    apply_command_to_state(&mut state, RelayCommand::Unsubscribe { channel_id: left });
    for sub in [channel_sub_id(left), "attacker-chosen".to_owned()] {
        deliver_closed(&mut client, &mut state, &sub, DB_ERROR).await;
    }
    assert!(state.closed_failures.is_empty());
    assert!(state.rate_limited_pending.is_empty());
    assert!(!state.active_subscriptions.contains_key(&left));
}

#[tokio::test]
async fn unsubscribing_clears_the_failure_count() {
    let (mut client, _server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel);
    deliver_closed(&mut client, &mut state, &channel_sub_id(channel), DB_ERROR).await;
    apply_command_to_state(
        &mut state,
        RelayCommand::Unsubscribe {
            channel_id: channel,
        },
    );
    assert!(state.closed_failures.is_empty());
    assert!(state.rate_limited_pending.is_empty());
}

/// The review's regression: a quiet channel resumes from its `live_until`, the
/// reconnect REQ's history read fails (CLOSED, no EOSE), and the socket drops
/// before the retry. The next reconnect must ask for the same history again.
#[tokio::test]
async fn history_error_then_reconnect_replays_the_same_history() {
    let mut state = BgState::new();
    // A fixed wall clock, so the one-hour repair window can't move the cursor
    // between the two replays.
    state.now_secs = || 1_700_000_000;
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel); // base 1_000
    let sub = channel_sub_id(channel);
    let (mut live, _live_server) = test_ws_pair().await;
    deliver_eose(&mut live, &mut state, &sub).await;
    lose_connection(&mut state).await;

    let (mut client, _server, failed) = open_replay(&mut state, &sub).await;
    deliver_closed(&mut client, &mut state, &sub, DB_ERROR).await;
    lose_connection(&mut state).await;

    let since = first_replay_req(&mut state, &sub).await;
    assert_eq!(
        since, failed,
        "the replay after a failed history read must start where the failed one did"
    );
}

/// A subscription that reached EOSE and was then CLOSED stopped being live at
/// the CLOSED. A later socket close must not stamp `live_until` for it, or the
/// next replay would skip what was stored while it was closed.
#[tokio::test]
async fn closed_after_eose_does_not_stamp_live_until() {
    for message in [DB_ERROR, "rate-limited: retry in 30s"] {
        let mut state = BgState::new();
        let channel = Uuid::new_v4();
        seed_test_subscription(&mut state, channel);
        state.membership_sub_active = true;
        state.startup_watermark = Some(1_000);
        state.membership_last_seen = Some(1_000);
        let (mut client, _server) = test_ws_pair().await;
        for sub in [channel_sub_id(channel), MEMBERSHIP_NOTIF_SUB_ID.to_owned()] {
            deliver_eose(&mut client, &mut state, &sub).await;
            deliver_closed(&mut client, &mut state, &sub, message).await;
        }
        lose_connection(&mut state).await;
        assert!(
            state.live_until.is_empty(),
            "{message}: a CLOSED subscription must not be stamped live"
        );
        assert_eq!(state.channel_since(&channel), Some(1_000), "{message}");
        assert_eq!(state.membership_since(), Some(1_000), "{message}");
    }
}

/// A gap replay whose REQ the relay CLOSED never delivered the gap: the loss
/// record that REQ retired is restored at the CLOSED, so the retry on the same
/// socket (not only the next reconnect) is held at or before it, even with a
/// retained `live_until` past the gap.
#[tokio::test]
async fn closed_gap_replay_restores_its_loss_record_for_the_retry() {
    const LOST: u64 = 3_000;
    for arm in LossArm::both() {
        for message in [DB_ERROR, "rate-limited: retry in 30s"] {
            let mut state = arm.progress_past_a_loss(LOST).await;
            let sub = arm.sub();
            let (mut client, _server, first) = open_replay(&mut state, &sub).await;
            assert!(first <= LOST, "{sub} {message}: first replay since={first}");
            assert_eq!(arm.dropped(&state), None, "{sub} {message}");

            deliver_closed(&mut client, &mut state, &sub, message).await;
            assert_eq!(arm.dropped(&state), Some(LOST), "{sub} {message}");
            let since = arm.since(&state).unwrap();
            assert!(
                since <= LOST,
                "{sub} {message}: the retry must still cover {LOST}, got since={since}"
            );
        }
    }
}

/// Through the real socket-owner loop: a channel REQ answered with an error
/// CLOSED is retried after the first backoff step (2s ±20%), not before, with
/// the same `since`. The loop has no other traffic, so only the parked-retry
/// wake can send it in time.
#[tokio::test]
async fn socket_owner_retries_a_closed_req_after_the_backoff() {
    let (url, mut conns) = mock_relay().await;
    let keys = Keys::generate();
    let (ws, handshake) = do_connect(&url, &keys, None).await.unwrap();
    let mut conn = timeout(Duration::from_secs(5), conns.recv())
        .await
        .unwrap()
        .unwrap();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let (control_tx, _control_rx) = mpsc::channel(1);
    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    let task = tokio::spawn(run_background_task_with_state(
        BgState::new(),
        ws,
        handshake,
        event_tx,
        control_tx,
        cmd_rx,
        keys.clone(),
        url.clone(),
        keys.public_key().to_hex(),
        None,
    ));

    let channel = Uuid::new_v4();
    cmd_tx
        .send(RelayCommand::Subscribe {
            channel_id: channel,
            filter: test_channel_filter(),
            replay_since: Some(1_000),
        })
        .await
        .unwrap();
    let first = next_req(&mut conn).await;
    assert_eq!(first[1], channel_sub_id(channel));
    conn.ctl
        .send(MockCtl::Send(json!(["CLOSED", first[1], DB_ERROR])))
        .unwrap();
    let closed_at = tokio::time::Instant::now();

    let retry = next_req(&mut conn).await;
    let waited = closed_at.elapsed();
    assert!(
        waited >= Duration::from_millis(1_500),
        "retried after {waited:?}, before the 2s backoff"
    );
    assert!(waited <= Duration::from_secs(4), "retried after {waited:?}");
    assert_eq!(retry[1], first[1]);
    assert_eq!(
        retry[2]["since"], first[2]["since"],
        "the retry replays the same history"
    );

    cmd_tx.send(RelayCommand::Shutdown).await.unwrap();
    timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn next_parked_resubscribe_at_is_the_earliest_deadline() {
    let mut state = BgState::new();
    let now = tokio::time::Instant::now();
    assert_eq!(state.next_parked_resubscribe_at(), None);
    state
        .rate_limited_pending
        .insert(Uuid::new_v4(), now + Duration::from_secs(9));
    state.observer_resub_at = Some(now + Duration::from_secs(5));
    assert_eq!(
        state.next_parked_resubscribe_at(),
        Some(now + Duration::from_secs(5))
    );
    state.membership_resub_at = Some(now + Duration::from_secs(1));
    assert_eq!(
        state.next_parked_resubscribe_at(),
        Some(now + Duration::from_secs(1))
    );
}
