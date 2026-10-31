//! Replay cursor (gap retention across a dropped socket) and reconnect
//! spreading.
//!
//! Pure decisions are table-tested; the wiring (EOSE handling, close-code
//! capture, the reconnect entry, REQ `since` on the wire) is driven through the
//! production functions the reconnect path actually calls. No relay service.
use super::tests::{next_test_frame, seed_test_subscription, test_channel_filter, test_ws_pair};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};

#[test]
fn resume_since_table_over_input_combinations() {
    // (name, base, last_seen, dropped, expected)
    #[allow(clippy::type_complexity)]
    let cases: &[(&str, Option<u64>, Option<u64>, Option<u64>, Option<u64>)] = &[
        ("nothing", Some(1000), None, None, Some(1000)),
        ("no baseline", None, None, None, None),
        ("events", Some(1000), Some(2000), None, Some(2000)),
        ("gap only", Some(1000), None, Some(1500), Some(1500)),
        (
            "gap before events",
            Some(1000),
            Some(2000),
            Some(1500),
            Some(1500),
        ),
        (
            "events before gap",
            Some(1000),
            Some(2000),
            Some(2500),
            Some(2000),
        ),
    ];
    for (name, base, last_seen, dropped, expected) in cases {
        assert_eq!(
            resume_since(*base, *last_seen, *dropped),
            *expected,
            "{name}"
        );
    }
}

#[test]
fn jitter_maps_sample_onto_symmetric_twenty_percent_band() {
    let base = Duration::from_secs(100);
    for (sample, expected_ms) in [
        (0.0, 80_000),
        (0.25, 90_000),
        (0.5, 100_000),
        (0.75, 110_000),
        (1.0, 120_000),
        (-3.0, 80_000),
        (7.0, 120_000),
        (f64::NAN, 100_000),
    ] {
        assert_eq!(
            jittered_duration_with(base, sample).as_millis(),
            expected_ms,
            "sample {sample}"
        );
    }
}

#[test]
fn production_jitter_spans_the_whole_band() {
    let base = Duration::from_secs(100);
    let draws: Vec<_> = (0..400).map(|_| jittered_duration(base)).collect();
    assert!(draws
        .iter()
        .all(|d| *d >= Duration::from_secs(80) && *d <= Duration::from_secs(120)));
    // The old formula only produced 0.80x-0.89x.
    assert!(draws.iter().any(|d| *d > Duration::from_secs(110)));
    assert!(draws.iter().any(|d| *d < Duration::from_secs(90)));
}

#[test]
fn first_reconnect_delay_table() {
    let secs = Duration::from_secs;
    let cases: &[(Option<u16>, f64, Duration)] = &[
        (Some(1012), 0.0, Duration::ZERO),
        (Some(1012), 0.5, secs(5)),
        (Some(1012), 1.0, secs(10)),
        (Some(1012), 7.0, secs(10)),
        (Some(1012), f64::NAN, secs(5)),
        (Some(1001), 0.25, Duration::from_millis(2_500)),
        // Other closes keep today's immediate first attempt.
        (Some(1000), 0.5, Duration::ZERO),
        (Some(1006), 0.5, Duration::ZERO),
        (Some(1011), 0.5, Duration::ZERO),
        (None, 0.5, Duration::ZERO),
    ];
    for (code, sample, expected) in cases {
        assert_eq!(
            first_reconnect_delay(*code, *sample),
            *expected,
            "{code:?} sample {sample}"
        );
    }
}

#[tokio::test]
async fn close_frame_code_is_captured_by_the_message_handler() {
    let (mut client, _server) = test_ws_pair().await;
    let (event_tx, _event_rx) = mpsc::channel(1);
    let (control_tx, _control_rx) = mpsc::channel(1);
    let mut state = BgState::new();
    for (frame, expected) in [
        (
            Some(CloseFrame {
                code: CloseCode::Restart,
                reason: "".into(),
            }),
            Some(1012),
        ),
        (
            Some(CloseFrame {
                code: CloseCode::Away,
                reason: "".into(),
            }),
            Some(1001),
        ),
        (None, None),
    ] {
        let alive = handle_ws_message(
            Message::Close(frame),
            &mut client,
            &event_tx,
            &control_tx,
            &mut state,
            &Keys::generate(),
            "ws://127.0.0.1:1",
            "agent",
            None,
        )
        .await;
        assert!(!alive, "a Close frame ends the connection");
        assert_eq!(state.last_close_code, expected);
    }
}

#[test]
fn begin_reconnect_consumes_the_close_code() {
    let mut state = BgState::new();
    state.last_close_code = Some(1012);
    assert_eq!(begin_reconnect(&mut state, 0.5), Duration::from_secs(5));
    // A later reconnect for an unrelated reason must not inherit the delay.
    assert_eq!(begin_reconnect(&mut state, 0.5), Duration::ZERO);
}

async fn deliver_eose(client: &mut WsStream, state: &mut BgState, sub_id: &str) {
    let (event_tx, _event_rx) = mpsc::channel(1);
    let (control_tx, _control_rx) = mpsc::channel(1);
    let alive = handle_ws_message(
        Message::Text(json!(["EOSE", sub_id]).to_string().into()),
        client,
        &event_tx,
        &control_tx,
        state,
        &Keys::generate(),
        "ws://127.0.0.1:1",
        "agent",
        None,
    )
    .await;
    assert!(alive);
}

/// Wire `since` of a connected `SubscribeMembership` command for a membership
/// subscription with delivered progress at 4_000.
async fn subscribe_membership_wire_since(dropped: Option<u64>) -> u64 {
    let (mut client, mut server) = test_ws_pair().await;
    let mut state = BgState::new();
    state.startup_watermark = Some(1_000);
    state.membership_last_seen = Some(4_000);
    state.membership_dropped_since = dropped;
    assert!(
        execute_connected_command(
            &mut client,
            &mut state,
            "agent",
            RelayCommand::SubscribeMembership
        )
        .await
    );
    let req = next_test_frame(&mut server).await;
    assert_eq!(req[1], MEMBERSHIP_NOTIF_SUB_ID);
    req[2]["since"].as_u64().unwrap()
}

#[tokio::test]
async fn subscribe_membership_command_uses_the_membership_cursor() {
    // A recorded gap wins over delivered progress.
    assert_eq!(
        subscribe_membership_wire_since(Some(3_000)).await,
        3_000 - SINCE_SKEW_SECS
    );
    assert_eq!(
        subscribe_membership_wire_since(None).await,
        4_000 - SINCE_SKEW_SECS
    );
}

#[tokio::test]
async fn removing_a_channel_clears_its_retired_loss() {
    let (mut client, _server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel);
    state.channel_dropped_since.insert(channel, 3_000);
    let since = state.channel_since(&channel);
    assert!(
        send_subscribe(
            &mut client,
            &mut state,
            channel,
            "agent",
            since,
            &test_channel_filter()
        )
        .await
    );
    assert!(state
        .retired_loss_this_conn
        .contains_key(&channel_sub_id(channel)));

    apply_command_to_state(
        &mut state,
        RelayCommand::Unsubscribe {
            channel_id: channel,
        },
    );
    assert!(state.retired_loss_this_conn.is_empty());
    begin_reconnect(&mut state, 0.0);
    assert!(state.channel_dropped_since.is_empty());
}

// -- Reconnect entry points, driven through the production functions --------

struct ReconnectHarness {
    cmd_rx: mpsc::Receiver<RelayCommand>,
    event_tx: mpsc::Sender<Option<BuzzEvent>>,
    control_tx: mpsc::Sender<Event>,
    keys: Keys,
}

impl ReconnectHarness {
    /// A harness whose command queue already holds `Shutdown`, so any wait
    /// inside the reconnect path ends the call immediately.
    async fn with_queued_shutdown() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(4);
        let (event_tx, _) = mpsc::channel(1);
        let (control_tx, _) = mpsc::channel(1);
        cmd_tx.send(RelayCommand::Shutdown).await.unwrap();
        Self {
            cmd_rx,
            event_tx,
            control_tx,
            keys: Keys::generate(),
        }
    }

    async fn autonomous(&mut self, ws: &mut WsStream, state: &mut BgState, url: &str) {
        let outcome = try_autonomous_reconnect(
            ws,
            &mut self.cmd_rx,
            state,
            &self.keys,
            url,
            "agent",
            &self.event_tx,
            &self.control_tx,
            None,
        )
        .await;
        assert!(matches!(outcome, ReconnectOutcome::Shutdown));
    }
}

/// A relay URL nothing listens on: connects are refused immediately.
async fn refused_relay_url() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("ws://{addr}")
}

/// A TCP server that counts connections and hangs up immediately, so the
/// WebSocket handshake fails fast.
async fn counting_relay() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    (url, accepted, task)
}

#[tokio::test]
async fn wait_for_reconnect_restores_a_retired_loss_too() {
    let (mut dead, _dead_server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel);
    state
        .retired_loss_this_conn
        .insert(channel_sub_id(channel), 3_000);

    let url = refused_relay_url().await;
    let mut harness = ReconnectHarness::with_queued_shutdown().await;
    let outcome = wait_for_reconnect(
        &mut dead,
        &mut harness.cmd_rx,
        &mut state,
        &harness.keys,
        &url,
        "agent",
        &harness.event_tx,
        &harness.control_tx,
        true,
        None,
    )
    .await;
    assert!(matches!(outcome, ReconnectOutcome::Shutdown));
    assert_eq!(state.channel_dropped_since.get(&channel), Some(&3_000));
}

#[tokio::test]
async fn restart_closes_delay_the_first_attempt_and_other_closes_do_not() {
    for (close_code, expected_attempts) in [
        (Some(1012), 0),
        (Some(1001), 0),
        (Some(1000), 1),
        (Some(1006), 1),
        (None, 1),
    ] {
        let (mut dead, _server) = test_ws_pair().await;
        let (url, accepted, relay) = counting_relay().await;
        let mut state = BgState::new();
        state.last_close_code = close_code;
        ReconnectHarness::with_queued_shutdown()
            .await
            .autonomous(&mut dead, &mut state, &url)
            .await;
        assert_eq!(
            accepted.load(Ordering::SeqCst),
            expected_attempts,
            "close code {close_code:?}"
        );
        relay.abort();
    }
}

// -- Superseded-REQ EOSE ------------------------------------------------------

#[tokio::test]
async fn second_req_on_a_connection_supersedes_the_first_for_eose() {
    let (mut client, _server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel);
    let sub = channel_sub_id(channel);
    for expect_released in [true, false] {
        state.channel_dropped_since.insert(channel, 3_000);
        let since = state.channel_since(&channel);
        assert!(
            send_subscribe(
                &mut client,
                &mut state,
                channel,
                "agent",
                since,
                &test_channel_filter()
            )
            .await
        );
        state.channel_dropped_since.remove(&channel);
        deliver_eose(&mut client, &mut state, &sub).await;
        assert_eq!(
            !state.retired_loss_this_conn.contains_key(&sub),
            expect_released
        );
    }
    // A new connection starts clean: its first REQ counts again.
    begin_reconnect(&mut state, 0.0);
    assert!(state.req_sent_this_conn.is_empty() && state.superseded_this_conn.is_empty());
}

fn tagged_event(channel: Uuid, created_at: u64) -> Event {
    EventBuilder::new(Kind::Custom(9), format!("e{created_at}"))
        .tags([Tag::parse(["h", &channel.to_string()]).unwrap()])
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(&Keys::generate())
        .unwrap()
}

async fn deliver_event(
    client: &mut WsStream,
    state: &mut BgState,
    tx: &mpsc::Sender<Option<BuzzEvent>>,
    sub_id: &str,
    event: &Event,
) {
    let (control_tx, _control_rx) = mpsc::channel(1);
    let alive = handle_ws_message(
        Message::Text(json!(["EVENT", sub_id, event]).to_string().into()),
        client,
        tx,
        &control_tx,
        state,
        &Keys::generate(),
        "ws://127.0.0.1:1",
        "agent",
        None,
    )
    .await;
    assert!(alive);
}

/// Drop the live socket the way the production loop does: through the real
/// autonomous reconnect entry (queued Shutdown, unreachable relay).
async fn lose_connection(state: &mut BgState) {
    let (mut dead, _dead_server) = test_ws_pair().await;
    let url = refused_relay_url().await;
    ReconnectHarness::with_queued_shutdown()
        .await
        .autonomous(&mut dead, state, &url)
        .await;
}

/// Replay `state` on a fresh socket and return the `since` of the first REQ.
async fn first_replay_req(state: &mut BgState, expected_sub: &str) -> u64 {
    let (mut client, mut server) = test_ws_pair().await;
    let (_cmd_tx, mut cmd_rx) = mpsc::channel(1);
    let result = resubscribe_after_reconnect(&mut client, &mut cmd_rx, state, "agent", true).await;
    assert!(matches!(result, ResubscribeResult::Ok));
    let req = next_test_frame(&mut server).await;
    assert_eq!(req[1], expected_sub);
    req[2]["since"].as_u64().unwrap()
}

#[tokio::test]
async fn delayed_eose_of_a_superseded_req_keeps_the_channel_overflow_floor() {
    let (mut client, mut server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel); // base 1_000
    let sub = channel_sub_id(channel);
    let (tx, mut rx) = mpsc::channel(2);

    // REQ #1 is on the wire; its EOSE is still in flight.
    let since = state.channel_since(&channel);
    assert!(
        send_subscribe(
            &mut client,
            &mut state,
            channel,
            "agent",
            since,
            &test_channel_filter()
        )
        .await
    );
    assert_eq!(next_test_frame(&mut server).await[0], "REQ");

    // History arrives faster than the consumer drains it: the third event
    // overflows the queue and is recorded as lost.
    for ts in [1_010, 1_020, 1_100] {
        deliver_event(
            &mut client,
            &mut state,
            &tx,
            &sub,
            &tagged_event(channel, ts),
        )
        .await;
    }
    assert_eq!(state.channel_dropped_since[&channel], 1_100);
    while rx.try_recv().is_ok() {}
    // A newer event is delivered after the loss, so `last_seen` passes it.
    deliver_event(
        &mut client,
        &mut state,
        &tx,
        &sub,
        &tagged_event(channel, 1_200),
    )
    .await;
    assert_eq!(state.last_seen[&channel], 1_200);
    while rx.try_recv().is_ok() {}

    // Overflow recovery writes REQ #2 and retires the loss floor on the write.
    recovery::recover_one(&mut client, &mut state, &tx, "agent").await;
    assert_eq!(next_test_frame(&mut server).await[0], "REQ");
    assert!(!state.channel_dropped_since.contains_key(&channel));

    // The in-flight EOSE of REQ #1 lands, then the socket drops before REQ #2
    // has replayed the lost event.
    deliver_eose(&mut client, &mut state, &sub).await;
    lose_connection(&mut state).await;

    let since = first_replay_req(&mut state, &sub).await;
    assert!(
        since <= 1_100,
        "replay must still cover the dropped event at 1100, got since={since}"
    );
}

#[tokio::test]
async fn delayed_eose_of_a_superseded_req_keeps_the_membership_overflow_floor() {
    let (mut client, mut server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    state.membership_sub_active = true;
    state.startup_watermark = Some(1_000);
    state.membership_last_seen = Some(1_000);
    let (tx, mut rx) = mpsc::channel(2);

    let since = state.membership_since();
    assert!(send_membership_subscribe(&mut client, &mut state, "agent", since).await);
    assert_eq!(next_test_frame(&mut server).await[0], "REQ");

    for ts in [1_010, 1_020, 1_100] {
        deliver_event(
            &mut client,
            &mut state,
            &tx,
            MEMBERSHIP_NOTIF_SUB_ID,
            &tagged_event(channel, ts),
        )
        .await;
    }
    assert_eq!(state.membership_dropped_since, Some(1_100));
    while rx.try_recv().is_ok() {}
    deliver_event(
        &mut client,
        &mut state,
        &tx,
        MEMBERSHIP_NOTIF_SUB_ID,
        &tagged_event(channel, 1_200),
    )
    .await;
    assert_eq!(state.membership_last_seen, Some(1_200));
    while rx.try_recv().is_ok() {}

    recovery::recover_one(&mut client, &mut state, &tx, "agent").await;
    assert_eq!(next_test_frame(&mut server).await[0], "REQ");
    assert_eq!(state.membership_dropped_since, None);

    deliver_eose(&mut client, &mut state, MEMBERSHIP_NOTIF_SUB_ID).await;
    lose_connection(&mut state).await;

    let since = first_replay_req(&mut state, MEMBERSHIP_NOTIF_SUB_ID).await;
    assert!(
        since <= 1_100,
        "replay must still cover the dropped notification at 1100, got since={since}"
    );
}
// -- A gap replay that never reached EOSE keeps its floor -------------------

/// Reconnect, write the replay REQs, optionally deliver `newer` on that socket
/// (a partial replay), and drop the socket before any EOSE.
async fn replay_then_lose_before_eose(
    state: &mut BgState,
    sub: &str,
    newer: Option<&Event>,
) -> u64 {
    let (mut client, mut server) = test_ws_pair().await;
    let (_cmd_tx, mut cmd_rx) = mpsc::channel(1);
    let result = resubscribe_after_reconnect(&mut client, &mut cmd_rx, state, "agent", true).await;
    assert!(matches!(result, ResubscribeResult::Ok));
    let req = next_test_frame(&mut server).await;
    assert_eq!(req[1], sub);
    if let Some(event) = newer {
        let (tx, _rx) = mpsc::channel(8);
        deliver_event(&mut client, state, &tx, sub, event).await;
    }
    lose_connection(state).await;
    req[2]["since"].as_u64().unwrap()
}

/// How a newer delivered event (`last_seen` past the loss) is mixed into
/// [`unfinished_gap_replay_keeps_its_floor`].
#[derive(Clone, Copy)]
enum NewerEvent {
    None,
    /// Delivered before the loss is replayed: `last_seen` is already ahead.
    BeforeReplay,
    /// Delivered on the replay socket before it drops: a partial replay.
    DuringReplay,
}

/// The review's schedule: a subscription loses an event to backpressure, the
/// reconnect replay REQ correctly goes out below it (retiring the loss record),
/// and that socket drops before the replay's EOSE. A newer `last_seen` may not
/// skip the lost event on the next replay. Once a replay does reach EOSE, the
/// cursor moves past the gap again.
async fn unfinished_gap_replay_keeps_its_floor(
    state: &mut BgState,
    sub: &str,
    channel: Uuid,
    newer: NewerEvent,
    record_loss: impl FnOnce(&mut BgState),
    loss_recorded: impl Fn(&BgState) -> bool,
) {
    const LOST: u64 = 3_000;
    let newer_event = tagged_event(channel, 4_000);
    let (mut live, _live_server) = test_ws_pair().await;
    record_loss(state);
    if matches!(newer, NewerEvent::BeforeReplay) {
        let (tx, _rx) = mpsc::channel(8);
        deliver_event(&mut live, state, &tx, sub, &newer_event).await;
    }
    lose_connection(state).await;

    let during = matches!(newer, NewerEvent::DuringReplay).then_some(&newer_event);
    let first = replay_then_lose_before_eose(state, sub, during).await;
    assert!(first <= LOST, "first replay since={first}");

    for attempt in 0..2 {
        let since = replay_then_lose_before_eose(state, sub, during).await;
        assert!(
            since <= LOST,
            "replay {attempt} after an unfinished gap replay must still cover {LOST}, got since={since}"
        );
    }

    // This time the replay reaches EOSE before the socket drops.
    let (mut client, mut server) = test_ws_pair().await;
    let (_cmd_tx, mut cmd_rx) = mpsc::channel(1);
    let result = resubscribe_after_reconnect(&mut client, &mut cmd_rx, state, "agent", true).await;
    assert!(matches!(result, ResubscribeResult::Ok));
    assert_eq!(next_test_frame(&mut server).await[1], sub);
    assert!(
        !loss_recorded(state),
        "writing the replay retires the loss record"
    );
    deliver_eose(&mut client, state, sub).await;
    lose_connection(state).await;
    assert!(!loss_recorded(state), "a finished replay is not restored");
    let since = first_replay_req(state, sub).await;
    let progress = match newer {
        NewerEvent::None => 1_000,
        NewerEvent::BeforeReplay | NewerEvent::DuringReplay => 4_000,
    };
    assert_eq!(
        since,
        progress - SINCE_SKEW_SECS,
        "a completed replay must not keep the old floor"
    );
}

const NEWER_EVENT_CASES: [NewerEvent; 3] = [
    NewerEvent::None,
    NewerEvent::BeforeReplay,
    NewerEvent::DuringReplay,
];

#[tokio::test]
async fn unfinished_channel_gap_replay_keeps_its_floor() {
    for newer in NEWER_EVENT_CASES {
        let mut state = BgState::new();
        let channel = Uuid::new_v4();
        seed_test_subscription(&mut state, channel); // base 1_000
        unfinished_gap_replay_keeps_its_floor(
            &mut state,
            &channel_sub_id(channel),
            channel,
            newer,
            |state| {
                state.channel_dropped_since.insert(channel, 3_000);
            },
            |state| state.channel_dropped_since.contains_key(&channel),
        )
        .await;
    }
}

#[tokio::test]
async fn unfinished_membership_gap_replay_keeps_its_floor() {
    for newer in NEWER_EVENT_CASES {
        let mut state = BgState::new();
        state.membership_sub_active = true;
        state.startup_watermark = Some(1_000);
        state.membership_last_seen = Some(1_000);
        unfinished_gap_replay_keeps_its_floor(
            &mut state,
            MEMBERSHIP_NOTIF_SUB_ID,
            Uuid::new_v4(),
            newer,
            |state| state.membership_dropped_since = Some(3_000),
            |state| state.membership_dropped_since.is_some(),
        )
        .await;
    }
}
/// Write the reconnect replay on a fresh socket, leaving the connection open,
/// and return the socket pair with the first REQ's `since`.
async fn open_replay(
    state: &mut BgState,
    sub: &str,
) -> (WsStream, WebSocketStream<tokio::net::TcpStream>, u64) {
    let (mut client, mut server) = test_ws_pair().await;
    let (_cmd_tx, mut cmd_rx) = mpsc::channel(1);
    let result = resubscribe_after_reconnect(&mut client, &mut cmd_rx, state, "agent", true).await;
    assert!(matches!(result, ResubscribeResult::Ok));
    let req = next_test_frame(&mut server).await;
    assert_eq!(req[1], sub);
    let since = req[2]["since"].as_u64().unwrap();
    (client, server, since)
}

/// The loss record a gap-retention test drives: a channel's or the membership
/// subscription's.
#[derive(Clone, Copy)]
enum LossArm {
    Channel(Uuid),
    Membership,
}

impl LossArm {
    fn both() -> [Self; 2] {
        [Self::Channel(Uuid::new_v4()), Self::Membership]
    }

    fn sub(self) -> String {
        match self {
            Self::Channel(channel) => channel_sub_id(channel),
            Self::Membership => MEMBERSHIP_NOTIF_SUB_ID.to_owned(),
        }
    }

    fn event(self, created_at: u64) -> Event {
        match self {
            Self::Channel(channel) => tagged_event(channel, created_at),
            Self::Membership => tagged_event(Uuid::nil(), created_at),
        }
    }

    fn dropped(self, state: &BgState) -> Option<u64> {
        match self {
            Self::Channel(channel) => state.channel_dropped_since.get(&channel).copied(),
            Self::Membership => state.membership_dropped_since,
        }
    }

    /// Delivered progress at 4_000 past a recorded loss at `lost`, the socket
    /// already closed.
    async fn progress_past_a_loss(self, lost: u64) -> BgState {
        let mut state = BgState::new();
        match self {
            Self::Channel(channel) => {
                seed_test_subscription(&mut state, channel); // base 1_000
                state.last_seen.insert(channel, 4_000);
                state.channel_dropped_since.insert(channel, lost);
            }
            Self::Membership => {
                state.membership_sub_active = true;
                state.startup_watermark = Some(1_000);
                state.membership_last_seen = Some(4_000);
                state.membership_dropped_since = Some(lost);
            }
        }
        lose_connection(&mut state).await;
        state
    }

    /// Deliver `overflow` on `client` into `tx`, a consumer with room for two
    /// events: the last one is dropped and recorded as a loss.
    async fn overflow(
        self,
        client: &mut WsStream,
        state: &mut BgState,
        tx: &mpsc::Sender<Option<BuzzEvent>>,
        overflow: [u64; 3],
    ) {
        for ts in overflow {
            deliver_event(client, state, tx, &self.sub(), &self.event(ts)).await;
        }
        assert_eq!(self.dropped(state), Some(overflow[2]));
    }
}

/// A socket that dies partway through a resubscribe (`RetryConnection`) is
/// redialed by the reconnect loop without going back through the reconnect
/// entry. The next resubscribe must still restore the loss record the dead
/// socket's REQ retired, and that socket's first REQ must count its EOSE.
#[tokio::test]
async fn redial_after_a_failed_resubscribe_keeps_the_floor() {
    const LOST: u64 = 3_000;
    let arm = LossArm::Channel(Uuid::new_v4());
    let mut state = arm.progress_past_a_loss(LOST).await;
    let sub = arm.sub();

    let (_dead, _dead_server, first) = open_replay(&mut state, &sub).await;
    assert!(first <= LOST, "first replay since={first}");
    assert_eq!(arm.dropped(&state), None);

    // That socket dies before EOSE and the loop redials straight into another
    // resubscribe.
    let (mut client, _server, second) = open_replay(&mut state, &sub).await;
    assert!(
        second <= LOST,
        "redial after a failed resubscribe must still cover {LOST}, got since={second}"
    );
    deliver_eose(&mut client, &mut state, &sub).await;
    assert!(
        !state.retired_loss_this_conn.contains_key(&sub),
        "the redialed socket's REQ is not superseded by the dead socket's"
    );
}

/// A new loss recorded while an unfinished replay is in flight merges with
/// the restored floor by `min`, and closing the same outage again changes
/// nothing.
#[tokio::test]
async fn loss_during_an_unfinished_replay_merges_with_the_restored_floor() {
    const LOST: u64 = 3_000;
    for arm in LossArm::both() {
        for (overflow, expected) in [
            ([3_400, 3_450, 3_500], LOST),
            ([2_400, 2_450, 2_500], 2_500),
        ] {
            let mut state = arm.progress_past_a_loss(LOST).await;
            let sub = arm.sub();
            let (mut client, _server, since) = open_replay(&mut state, &sub).await;
            assert!(since <= LOST);
            let (tx, _rx) = mpsc::channel(2);
            arm.overflow(&mut client, &mut state, &tx, overflow).await;

            lose_connection(&mut state).await;
            assert_eq!(arm.dropped(&state), Some(expected));

            // A repeated close for the same outage is a no-op.
            begin_reconnect(&mut state, 0.0);
            assert_eq!(arm.dropped(&state), Some(expected));

            let since = first_replay_req(&mut state, &sub).await;
            assert!(
                since <= expected,
                "replay must cover the earliest loss {expected}, got since={since}"
            );
        }
    }
}

/// History replays newest first, so a loss recorded during the reconnect
/// replay can be older than the loss that replay retired. Overflow recovery's
/// second REQ on the same socket retires it too; if the socket drops before
/// EOSE, the older of the two retired losses is restored.
#[tokio::test]
async fn a_second_replay_on_one_socket_restores_the_older_retired_loss() {
    const LOST: u64 = 3_000;
    const OLDER: u64 = 2_500;
    for arm in LossArm::both() {
        let mut state = arm.progress_past_a_loss(LOST).await;
        let sub = arm.sub();
        let (mut client, mut server, since) = open_replay(&mut state, &sub).await;
        assert!(since <= LOST);
        let (tx, mut rx) = mpsc::channel(2);
        arm.overflow(&mut client, &mut state, &tx, [2_400, 2_450, OLDER])
            .await;
        while rx.try_recv().is_ok() {}

        // Overflow recovery writes REQ #2 and retires the older loss on the write.
        recovery::recover_one(&mut client, &mut state, &tx, "agent").await;
        assert_eq!(next_test_frame(&mut server).await[1], sub.as_str());
        assert_eq!(arm.dropped(&state), None);

        lose_connection(&mut state).await;
        assert_eq!(arm.dropped(&state), Some(OLDER));
        assert_eq!(
            first_replay_req(&mut state, &sub).await,
            OLDER - SINCE_SKEW_SECS
        );
    }
}
