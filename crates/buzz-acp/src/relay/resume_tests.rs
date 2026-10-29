//! Resume cursor (EOSE-derived `live_until`) and reconnect spreading.
//!
//! Pure decisions are table-tested; the wiring (EOSE handling, close-code
//! capture, the reconnect entry, REQ `since` on the wire) is driven through the
//! production functions the reconnect path actually calls. No relay service.
use super::tests::{next_test_frame, seed_test_subscription, test_channel_filter, test_ws_pair};
use super::*;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};

const SUB_FLOOR_OFFSET: u64 = SILENT_DROP_REPAIR_WINDOW_SECS;
/// `live_until` used by the table, and the live floor it gives.
const LIVE: u64 = 10_000;
const FLOOR: u64 = LIVE - SUB_FLOOR_OFFSET;

#[test]
fn resume_since_table_over_input_combinations() {
    // (name, base, last_seen, dropped, live_until, expected)
    #[allow(clippy::type_complexity)]
    let cases: &[(
        &str,
        Option<u64>,
        Option<u64>,
        Option<u64>,
        Option<u64>,
        Option<u64>,
    )] = &[
        // No EOSE on the closed connection: today's behavior.
        ("no eose, nothing", Some(1000), None, None, None, Some(1000)),
        ("no eose, no baseline", None, None, None, None, None),
        (
            "no eose, events",
            Some(1000),
            Some(2000),
            None,
            None,
            Some(2000),
        ),
        (
            "no eose, dropped only",
            Some(1000),
            None,
            Some(1500),
            None,
            Some(1500),
        ),
        (
            "no eose, dropped below last_seen",
            Some(1000),
            Some(2000),
            Some(1500),
            None,
            Some(1500),
        ),
        (
            "no eose, dropped above last_seen",
            Some(1000),
            Some(2000),
            Some(2500),
            None,
            Some(2000),
        ),
        // EOSE and no events: a quiet sub resumes from live_until, not the baseline.
        (
            "eose, quiet",
            Some(1000),
            None,
            None,
            Some(LIVE),
            Some(FLOOR),
        ),
        (
            "eose, quiet, no baseline",
            None,
            None,
            None,
            Some(LIVE),
            Some(FLOOR),
        ),
        // EOSE and events: newest of last_seen and the live floor wins.
        (
            "eose, last_seen newer than floor",
            Some(1000),
            Some(LIVE - 10),
            None,
            Some(LIVE),
            Some(LIVE - 10),
        ),
        (
            "eose, floor newer than last_seen",
            Some(1000),
            Some(FLOOR - 100),
            None,
            Some(LIVE),
            Some(FLOOR),
        ),
        // Dropped / gap floor keeps precedence over the live cursor.
        (
            "eose, dropped below cursor",
            Some(1000),
            Some(LIVE - 10),
            Some(3000),
            Some(LIVE),
            Some(3000),
        ),
        (
            "eose, dropped only",
            Some(1000),
            None,
            Some(3000),
            Some(LIVE),
            Some(3000),
        ),
        (
            "eose, dropped above cursor",
            Some(1000),
            None,
            Some(LIVE - 10),
            Some(LIVE),
            Some(FLOOR),
        ),
        // Clock-skew edges: never below the original floor, never underflow.
        (
            "eose, live_until before baseline",
            Some(1000),
            None,
            None,
            Some(30),
            Some(1000),
        ),
        (
            "eose, live_until barely above baseline",
            Some(1000),
            None,
            None,
            Some(1100),
            Some(1000),
        ),
        (
            "eose, live_until underflow, no baseline",
            None,
            None,
            None,
            Some(0),
            Some(0),
        ),
    ];
    for (name, base, last_seen, dropped, live_until, expected) in cases {
        assert_eq!(
            resume_since(*base, *last_seen, *dropped, *live_until),
            *expected,
            "{name}"
        );
    }
}

#[test]
fn repair_window_covers_commit_delay_and_clock_skew() {
    // The commit-time floor (960s, which already includes 60s of commit delay
    // on top of the 900s ingest drift) and the client clock skew (60s) are
    // separate budgets, and the repair window covers both (also asserted at
    // compile time); `send_subscribe` subtracts SINCE_SKEW_SECS on top.
    assert_eq!(RESUME_LIVE_SKEW_SECS, 960 + 60);
    let cursor = resume_since(Some(1), None, None, Some(10_000)).unwrap();
    assert_eq!(cursor, 10_000 - SILENT_DROP_REPAIR_WINDOW_SECS);
    assert!(10_000 - cursor >= RESUME_LIVE_SKEW_SECS);
}

#[test]
fn repair_window_counts_back_from_live_until() {
    const NOW: u64 = 10_000_000;
    let since = |base, last_seen, floor, live_until| {
        resume_since(base, last_seen, floor, Some(live_until)).unwrap()
    };
    // A short outage: the hour before the subscription went offline is
    // re-read, so an unsignaled live drop within it is repaired.
    assert_eq!(
        since(Some(1_000), None, None, NOW - 30),
        NOW - 30 - SILENT_DROP_REPAIR_WINDOW_SECS
    );
    // A long outage (a laptop sleep): still the full hour before it went
    // offline, not just the commit-floor margin, so a drop 30 minutes before
    // the sleep is repaired too.
    let live_until = NOW - 7_200;
    let drop_before_sleep = live_until - 1_800;
    assert_eq!(
        since(Some(1_000), None, None, live_until),
        live_until - SILENT_DROP_REPAIR_WINDOW_SECS
    );
    assert!(since(Some(1_000), None, None, live_until) <= drop_before_sleep);
    // Never before the original floor, which may be inside the window.
    assert_eq!(since(Some(NOW - 600), None, None, NOW - 30), NOW - 600);
    // Delivered progress still advances the cursor as it always did; the
    // window only bounds the live-derived term.
    assert_eq!(since(Some(1_000), Some(NOW - 60), None, NOW - 30), NOW - 60);
    // A recorded gap older than the window still wins.
    assert_eq!(since(Some(1_000), None, Some(5_000), NOW - 30), 5_000);
    // Without live_until the window plays no part: today's behavior.
    assert_eq!(resume_since(Some(1_000), None, None, None), Some(1_000));
}

#[tokio::test]
async fn quiet_subscription_resumed_now_replays_the_last_hour() {
    let (mut client, _server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel); // base 1_000
    deliver_eose(&mut client, &mut state, &channel_sub_id(channel)).await;
    let before = unix_now_secs();
    lose_connection(&mut state).await;
    let since = first_replay_req(&mut state, &channel_sub_id(channel)).await;
    let after = unix_now_secs();
    let window = SILENT_DROP_REPAIR_WINDOW_SECS + SINCE_SKEW_SECS;
    assert!(
        (before - window..=after - window).contains(&since),
        "since={since} must be one repair window before the REQ ({before}..={after})"
    );
}

/// Wire `since` of the replay for a quiet subscription whose connection was
/// last live at `live_until` on the agent's clock.
fn wire_since_after(live_until: u64) -> u64 {
    resume_since(Some(1_000), None, None, Some(live_until)).unwrap() - SINCE_SKEW_SECS
}

#[test]
fn backdated_event_committed_after_close_under_clock_skew_is_replayed() {
    // Server time T is when the relay sent its last frame.
    const T: u64 = 10_000_000;
    // The review's schedule: the agent clock runs +50s (inside NIP-42's ±60s),
    // an event passes ingest at T-30 dated T-930 (inside ±900s), and commits
    // at T+1 (inside the 960s commit floor), after the socket died.
    let (created_at, committed_at) = (T - 930, T + 1);
    assert!(committed_at - created_at <= RELAY_COMMIT_CREATED_AT_FLOOR_SECS);
    assert!(wire_since_after(T + 50) <= created_at);

    // The extreme of every budget at once: +60s skew, and an event dated the
    // full commit floor before a commit at T.
    let oldest = T - RELAY_COMMIT_CREATED_AT_FLOOR_SECS;
    assert!(wire_since_after(T + CLIENT_CLOCK_SKEW_SECS) <= oldest);
    // A clock running slow only widens the window.
    assert!(wire_since_after(T - CLIENT_CLOCK_SKEW_SECS) <= oldest);

    // The window is still bounded: a month-old event is not replayed.
    assert!(T - 30 * 86_400 < wire_since_after(T));
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
    assert_eq!(
        begin_reconnect(&mut state, 5_000, 0.5),
        Duration::from_secs(5)
    );
    // A later reconnect for an unrelated reason must not inherit the delay.
    assert_eq!(begin_reconnect(&mut state, 5_001, 0.5), Duration::ZERO);
}

pub(super) async fn deliver_eose(client: &mut WsStream, state: &mut BgState, sub_id: &str) {
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
        None,
    )
    .await;
    assert!(alive);
}

#[tokio::test]
async fn eose_tracking_only_covers_owned_subscriptions() {
    let (mut client, _server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel);
    for sub in [
        channel_sub_id(channel),
        MEMBERSHIP_NOTIF_SUB_ID.to_owned(),
        channel_sub_id(Uuid::new_v4()),
        OBSERVER_CONTROL_SUB_ID.to_owned(),
        "attacker-chosen".to_owned(),
    ] {
        deliver_eose(&mut client, &mut state, &sub).await;
    }
    state.note_connection_closed(5_000);
    let mut stamped: Vec<_> = state.live_until.keys().cloned().collect();
    stamped.sort();
    let mut expected = vec![channel_sub_id(channel), MEMBERSHIP_NOTIF_SUB_ID.to_owned()];
    expected.sort();
    assert_eq!(stamped, expected);
    assert!(state.eose_this_conn.is_empty());
}

#[tokio::test]
async fn quiet_channel_and_membership_resume_from_live_until() {
    let (mut client, _server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel); // subscribe_since = 1_000
    state.startup_watermark = Some(1_000);
    state.membership_last_seen = Some(1_000);
    state.membership_sub_active = true;
    assert_eq!(state.channel_since(&channel), Some(1_000));

    deliver_eose(&mut client, &mut state, &channel_sub_id(channel)).await;
    deliver_eose(&mut client, &mut state, MEMBERSHIP_NOTIF_SUB_ID).await;
    begin_reconnect(&mut state, 5_000, 0.0);

    assert_eq!(
        state.channel_since(&channel),
        Some(5_000 - SUB_FLOOR_OFFSET)
    );
    assert_eq!(state.membership_since(), Some(5_000 - SUB_FLOOR_OFFSET));

    // A recorded gap below the live floor still wins for both.
    state.channel_dropped_since.insert(channel, 1_200);
    state.membership_dropped_since = Some(1_200);
    assert_eq!(state.channel_since(&channel), Some(1_200));
    assert_eq!(state.membership_since(), Some(1_200));
}

/// Wire `since` of a connected `SubscribeMembership` command for a membership
/// subscription retained live until `LIVE`, with delivered progress at 4_000.
async fn subscribe_membership_wire_since(dropped: Option<u64>) -> u64 {
    let (mut client, mut server) = test_ws_pair().await;
    let mut state = BgState::new();
    state.startup_watermark = Some(1_000);
    state.membership_last_seen = Some(4_000);
    state.membership_dropped_since = dropped;
    state
        .live_until
        .insert(MEMBERSHIP_NOTIF_SUB_ID.to_owned(), LIVE);
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
    // A recorded gap wins over progress and live coverage.
    assert_eq!(
        subscribe_membership_wire_since(Some(3_000)).await,
        3_000 - SINCE_SKEW_SECS
    );
    // Without one, retained live coverage beats older delivered progress.
    assert_eq!(
        subscribe_membership_wire_since(None).await,
        FLOOR - SINCE_SKEW_SECS
    );
}

#[tokio::test]
async fn closed_connection_without_eose_keeps_the_previous_cursor() {
    let (mut client, _server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel);
    deliver_eose(&mut client, &mut state, &channel_sub_id(channel)).await;
    begin_reconnect(&mut state, 5_000, 0.0);
    // The next connection never reaches EOSE before it closes too.
    begin_reconnect(&mut state, 9_000, 0.0);
    assert_eq!(
        state.channel_since(&channel),
        Some(5_000 - SUB_FLOOR_OFFSET)
    );
    // Once a connection does reach EOSE, the cursor moves on.
    deliver_eose(&mut client, &mut state, &channel_sub_id(channel)).await;
    begin_reconnect(&mut state, 9_500, 0.0);
    assert_eq!(
        state.channel_since(&channel),
        Some(9_500 - SUB_FLOOR_OFFSET)
    );
}

#[tokio::test]
async fn resending_a_req_withdraws_its_eose() {
    let (mut client, _server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel);
    state.membership_sub_active = true;
    deliver_eose(&mut client, &mut state, &channel_sub_id(channel)).await;
    deliver_eose(&mut client, &mut state, MEMBERSHIP_NOTIF_SUB_ID).await;

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
    assert!(send_membership_subscribe(&mut client, &mut state, "agent", Some(1_000)).await);

    // Neither re-sent REQ has reached EOSE, so closing now proves nothing.
    begin_reconnect(&mut state, 5_000, 0.0);
    assert!(state.live_until.is_empty());
    assert_eq!(state.channel_since(&channel), Some(1_000));
}

#[tokio::test]
async fn removing_a_channel_clears_its_resume_state() {
    let (mut client, _server) = test_ws_pair().await;
    let mut state = BgState::new();
    let (stamped, pending) = (Uuid::new_v4(), Uuid::new_v4());
    seed_test_subscription(&mut state, stamped);
    seed_test_subscription(&mut state, pending);
    deliver_eose(&mut client, &mut state, &channel_sub_id(stamped)).await;
    begin_reconnect(&mut state, 5_000, 0.0);
    deliver_eose(&mut client, &mut state, &channel_sub_id(pending)).await;

    for channel in [stamped, pending] {
        apply_command_to_state(
            &mut state,
            RelayCommand::Unsubscribe {
                channel_id: channel,
            },
        );
    }
    assert!(state.live_until.is_empty());
    assert!(state.eose_this_conn.is_empty());
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
async fn autonomous_reconnect_stamps_live_until_and_replay_req_resumes_from_it() {
    let (mut live, _live_server) = test_ws_pair().await;
    let (mut dead, _dead_server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel); // original since = 1_000
    deliver_eose(&mut live, &mut state, &channel_sub_id(channel)).await;

    let before = unix_now_secs();
    let url = refused_relay_url().await;
    ReconnectHarness::with_queued_shutdown()
        .await
        .autonomous(&mut dead, &mut state, &url)
        .await;
    let after = unix_now_secs();

    let live_until = state.live_until[&channel_sub_id(channel)];
    assert!((before..=after).contains(&live_until));
    let since = first_replay_req(&mut state, &channel_sub_id(channel)).await;
    let expected = live_until - SILENT_DROP_REPAIR_WINDOW_SECS - SINCE_SKEW_SECS;
    assert_eq!(since, expected);
    assert!(since > 1_000, "must not replay from the original floor");
    assert!(
        since <= live_until - RELAY_COMMIT_CREATED_AT_FLOOR_SECS - CLIENT_CLOCK_SKEW_SECS,
        "a backdated event committed just after the close must be replayed"
    );
}

#[tokio::test]
async fn wait_for_reconnect_stamps_live_until_too() {
    let (mut live, _live_server) = test_ws_pair().await;
    let (mut dead, _dead_server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel);
    deliver_eose(&mut live, &mut state, &channel_sub_id(channel)).await;

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
    assert!(state.live_until.contains_key(&channel_sub_id(channel)));
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

// -- Liveness cap and superseded-REQ EOSE -----------------------------------

#[test]
fn live_until_is_capped_at_the_last_inbound_frame() {
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel);
    state.eose_this_conn.insert(channel_sub_id(channel));
    state.last_inbound_secs = Some(5_000);

    // The loss is only detected an hour later; the stamp is the last frame.
    begin_reconnect(&mut state, 5_000 + 3_600, 0.0);
    assert_eq!(state.live_until[&channel_sub_id(channel)], 5_000);
    // Kept for a repeated reconnect call of the same outage.
    assert_eq!(state.last_inbound_secs, Some(5_000));

    // A clock that went backwards never produces a stamp after "now".
    state.eose_this_conn.insert(channel_sub_id(channel));
    state.live_until.clear();
    state.last_inbound_secs = Some(9_000);
    begin_reconnect(&mut state, 8_000, 0.0);
    assert_eq!(state.live_until[&channel_sub_id(channel)], 8_000);
}

#[tokio::test]
async fn second_req_on_a_connection_supersedes_the_first_for_eose() {
    let (mut client, _server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel);
    let sub = channel_sub_id(channel);
    for expect_tracked in [true, false] {
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
        deliver_eose(&mut client, &mut state, &sub).await;
        assert_eq!(state.eose_this_conn.contains(&sub), expect_tracked);
    }
    // A new connection starts clean: its first REQ counts again.
    begin_reconnect(&mut state, 5_000, 0.0);
    assert!(state.req_sent_this_conn.is_empty() && state.superseded_this_conn.is_empty());
}

pub(super) fn tagged_event(channel: Uuid, created_at: u64) -> Event {
    EventBuilder::new(Kind::Custom(9), format!("e{created_at}"))
        .tags([Tag::parse(["h", &channel.to_string()]).unwrap()])
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(&Keys::generate())
        .unwrap()
}

pub(super) async fn deliver_event(
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
        None,
    )
    .await;
    assert!(alive);
}

/// Drop the live socket the way the production loop does: through the real
/// autonomous reconnect entry (queued Shutdown, unreachable relay).
pub(super) async fn lose_connection(state: &mut BgState) {
    let (mut dead, _dead_server) = test_ws_pair().await;
    let url = refused_relay_url().await;
    ReconnectHarness::with_queued_shutdown()
        .await
        .autonomous(&mut dead, state, &url)
        .await;
}

/// Replay `state` on a fresh socket and return the `since` of the first REQ.
pub(super) async fn first_replay_req(state: &mut BgState, expected_sub: &str) -> u64 {
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

/// The review's schedule: a subscription with a retained `live_until` loses an
/// event to backpressure, the reconnect replay REQ correctly goes out below it
/// (retiring the loss record), and that socket drops before the replay's EOSE.
/// Neither the retained `live_until` nor a newer `last_seen` may skip the lost
/// event on the next replay. Once a replay does reach EOSE, the cursor moves
/// past the gap again.
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
    deliver_eose(&mut live, state, sub).await;
    record_loss(state);
    if matches!(newer, NewerEvent::BeforeReplay) {
        let (tx, _rx) = mpsc::channel(8);
        deliver_event(&mut live, state, &tx, sub, &newer_event).await;
    }
    lose_connection(state).await;
    assert!(state.live_until[sub] > LOST + RESUME_LIVE_SKEW_SECS);

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
    assert!(
        since > LOST + RESUME_LIVE_SKEW_SECS,
        "a completed replay must not keep the old floor, got since={since}"
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

/// Overflow recovery on a connection whose subscription already has a retained
/// `live_until`: the recovery REQ retires the loss record when written, and the
/// socket drops before its replay finishes. The next replay must still cover
/// the lost event rather than resume from the retained live coverage or the
/// newer `last_seen`.
#[tokio::test]
async fn overflow_recovery_below_a_retained_live_until_keeps_its_floor() {
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    seed_test_subscription(&mut state, channel); // base 1_000
    let sub = channel_sub_id(channel);
    let (mut earlier, _earlier_server) = test_ws_pair().await;
    deliver_eose(&mut earlier, &mut state, &sub).await;
    lose_connection(&mut state).await;
    assert!(state.live_until.contains_key(&sub));

    // The reconnect replay reaches EOSE on the new socket.
    let (mut client, mut server) = test_ws_pair().await;
    let (_cmd_tx, mut cmd_rx) = mpsc::channel(1);
    let result =
        resubscribe_after_reconnect(&mut client, &mut cmd_rx, &mut state, "agent", true).await;
    assert!(matches!(result, ResubscribeResult::Ok));
    assert_eq!(next_test_frame(&mut server).await[1], sub);
    deliver_eose(&mut client, &mut state, &sub).await;

    // Live events overflow the queue; the third is recorded as lost.
    let (tx, mut rx) = mpsc::channel(2);
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

    recovery::recover_one(&mut client, &mut state, &tx, "agent").await;
    assert_eq!(next_test_frame(&mut server).await[0], "REQ");
    assert!(!state.channel_dropped_since.contains_key(&channel));

    lose_connection(&mut state).await;
    let since = first_replay_req(&mut state, &sub).await;
    assert!(
        since <= 1_100,
        "replay must still cover the dropped event at 1100, got since={since}"
    );
}

/// Write the reconnect replay on a fresh socket, leaving the connection open,
/// and return the socket pair with the first REQ's `since`.
pub(super) async fn open_replay(
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
pub(super) enum LossArm {
    Channel(Uuid),
    Membership,
}

impl LossArm {
    pub(super) fn both() -> [Self; 2] {
        [Self::Channel(Uuid::new_v4()), Self::Membership]
    }

    pub(super) fn sub(self) -> String {
        match self {
            Self::Channel(channel) => channel_sub_id(channel),
            Self::Membership => MEMBERSHIP_NOTIF_SUB_ID.to_owned(),
        }
    }

    pub(super) fn event(self, created_at: u64) -> Event {
        match self {
            Self::Channel(channel) => tagged_event(channel, created_at),
            Self::Membership => tagged_event(Uuid::nil(), created_at),
        }
    }

    pub(super) fn dropped(self, state: &BgState) -> Option<u64> {
        match self {
            Self::Channel(channel) => state.channel_dropped_since.get(&channel).copied(),
            Self::Membership => state.membership_dropped_since,
        }
    }

    /// The `since` this subscription's next REQ would carry.
    pub(super) fn since(self, state: &BgState) -> Option<u64> {
        match self {
            Self::Channel(channel) => state.channel_since(&channel),
            Self::Membership => state.membership_since(),
        }
    }

    /// Delivered progress at 4_000 and a retained `live_until` past a
    /// recorded loss at `lost`, the socket already closed.
    pub(super) async fn progress_past_a_loss(self, lost: u64) -> BgState {
        let mut state = BgState::new();
        match self {
            Self::Channel(channel) => {
                seed_test_subscription(&mut state, channel); // base 1_000
                state.last_seen.insert(channel, 4_000);
            }
            Self::Membership => {
                state.membership_sub_active = true;
                state.startup_watermark = Some(1_000);
                state.membership_last_seen = Some(4_000);
            }
        }
        let (mut live, _live_server) = test_ws_pair().await;
        deliver_eose(&mut live, &mut state, &self.sub()).await;
        match self {
            Self::Channel(channel) => {
                state.channel_dropped_since.insert(channel, lost);
            }
            Self::Membership => state.membership_dropped_since = Some(lost),
        }
        lose_connection(&mut state).await;
        assert!(state.live_until[&self.sub()] > lost + RESUME_LIVE_SKEW_SECS);
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
        state.eose_this_conn.contains(&sub),
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
            let live_until = state.live_until[&sub];

            // A repeated close for the same outage is a no-op.
            let now = (state.now_secs)();
            begin_reconnect(&mut state, now, 0.0);
            assert_eq!(arm.dropped(&state), Some(expected));
            assert_eq!(state.live_until[&sub], live_until);

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

// -- Silent socket death, through the real socket-owner loop ----------------

pub(super) enum MockFrame {
    Text(Value),
    Pong,
    /// The agent's own keepalive Ping. tungstenite queues an auto-Pong for it,
    /// which is only written out by the mock's next write.
    Ping,
}

pub(super) enum MockCtl {
    Send(Value),
    Ping,
    Drop,
    /// Send a Close frame with this code, then hang up.
    Close(u16),
}

pub(super) struct MockConn {
    pub(super) frames: mpsc::UnboundedReceiver<MockFrame>,
    pub(super) ctl: mpsc::UnboundedSender<MockCtl>,
}

/// A relay that completes NIP-42 auth on every connection, surfaces each
/// authenticated connection to the test, and forwards its inbound frames.
pub(super) async fn mock_relay() -> (String, mpsc::UnboundedReceiver<MockConn>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (conn_tx, conn_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let conn_tx = conn_tx.clone();
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                let _ = ws
                    .send(Message::Text(
                        json!(["AUTH", "challenge"]).to_string().into(),
                    ))
                    .await;
                loop {
                    let Some(Ok(Message::Text(text))) = ws.next().await else {
                        return;
                    };
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    if frame[0] == "AUTH" {
                        let ok = json!(["OK", frame[1]["id"], true, ""]);
                        let _ = ws.send(Message::Text(ok.to_string().into())).await;
                        break;
                    }
                }
                let (frames_tx, frames_rx) = mpsc::unbounded_channel();
                let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel();
                let _ = conn_tx.send(MockConn {
                    frames: frames_rx,
                    ctl: ctl_tx,
                });
                loop {
                    tokio::select! {
                        msg = ws.next() => match msg {
                            Some(Ok(Message::Text(text))) => {
                                let _ = frames_tx.send(MockFrame::Text(
                                    serde_json::from_str(&text).unwrap(),
                                ));
                            }
                            Some(Ok(Message::Pong(_))) => {
                                let _ = frames_tx.send(MockFrame::Pong);
                            }
                            Some(Ok(Message::Ping(_))) => {
                                let _ = frames_tx.send(MockFrame::Ping);
                            }
                            Some(Ok(_)) => {}
                            _ => return,
                        },
                        ctl = ctl_rx.recv() => match ctl {
                            Some(MockCtl::Send(v)) => {
                                let _ = ws.send(Message::Text(v.to_string().into())).await;
                            }
                            Some(MockCtl::Ping) => {
                                let _ = ws.send(Message::Ping(vec![1].into())).await;
                            }
                            Some(MockCtl::Close(code)) => {
                                let frame = CloseFrame {
                                    code: CloseCode::from(code),
                                    reason: "".into(),
                                };
                                let _ = ws.send(Message::Close(Some(frame))).await;
                                return;
                            }
                            Some(MockCtl::Drop) | None => return,
                        },
                    }
                }
            });
        }
    });
    (url, conn_rx)
}

static FAKE_NOW: AtomicU64 = AtomicU64::new(0);
/// `FAKE_NOW` is process-global, so every test that moves it holds this lock
/// for its whole run; otherwise parallel threads corrupt each other's clock.
/// A tokio mutex: safe to hold across `.await` and not poisoned by a panic.
static FAKE_CLOCK_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn fake_now() -> u64 {
    FAKE_NOW.load(Ordering::SeqCst)
}

pub(super) async fn next_req(conn: &mut MockConn) -> Value {
    loop {
        let frame = timeout(Duration::from_secs(5), conn.frames.recv())
            .await
            .expect("timed out waiting for a REQ")
            .expect("mock connection closed");
        if let MockFrame::Text(v) = frame {
            if v[0] == "REQ" {
                return v;
            }
        }
    }
}

/// Waits until `reqs` REQs and the agent's keepalive Ping have all reached the
/// mock. The keepalive interval ticks immediately on start, so the relay owes
/// the agent an auto-Pong; once the mock has read that Ping, its next scripted
/// write flushes the Pong ahead of itself. Without this the Pong can arrive
/// after the test moves the clock and be stamped as a frame from the future.
async fn next_reqs_and_keepalive(conn: &mut MockConn, reqs: usize) {
    let (mut seen_reqs, mut seen_ping) = (0, false);
    while seen_reqs < reqs || !seen_ping {
        let frame = timeout(Duration::from_secs(5), conn.frames.recv())
            .await
            .expect("timed out waiting for REQs and the keepalive Ping")
            .expect("mock connection closed");
        match frame {
            MockFrame::Text(v) if v[0] == "REQ" => seen_reqs += 1,
            MockFrame::Ping => seen_ping = true,
            _ => {}
        }
    }
}

/// An agent reaches EOSE, the machine sleeps for two hours while the socket
/// dies, and the loss is only noticed on wake. Replay must resume from the
/// last frame the relay actually sent, not from the wake time.
#[tokio::test]
async fn silent_socket_death_resumes_from_last_inbound_frame_not_detection_time() {
    resume_after_a_two_hour_sleep(None, MockCtl::Drop).await;
}

/// Same outage, but a frame the relay sent just before the sleep sat in the
/// socket buffer and is read on wake, before the loss is noticed. It proves
/// nothing about the two hours the machine was asleep, so it must not stamp the
/// wake time. The fake wall clock jumps two hours; the monotonic clock, like a
/// suspended machine's, barely moves.
#[tokio::test]
async fn frame_buffered_across_a_sleep_does_not_stamp_wake_time() {
    let buffered = MockCtl::Send(json!(["NOTICE", "queued before the sleep"]));
    resume_after_a_two_hour_sleep(Some(buffered), MockCtl::Drop).await;
}

/// Same outage, but the relay's Close frame (rolling restart) sat in the
/// kernel buffer across the sleep and is the first thing read on wake. A Close
/// frame is not proof of liveness, so it must not move the stamp to wake time.
#[tokio::test]
async fn close_frame_read_after_a_sleep_does_not_stamp_wake_time() {
    resume_after_a_two_hour_sleep(None, MockCtl::Close(1012)).await;
}

async fn resume_after_a_two_hour_sleep(buffered: Option<MockCtl>, loss: MockCtl) {
    let _clock = FAKE_CLOCK_LOCK.lock().await;
    const T0: u64 = 1_700_000_000;
    FAKE_NOW.store(T0, Ordering::SeqCst);
    let (url, mut conns) = mock_relay().await;
    let keys = Keys::generate();
    let (ws, handshake) = do_connect(&url, &keys, None).await.unwrap();
    let mut conn0 = timeout(Duration::from_secs(5), conns.recv())
        .await
        .unwrap()
        .unwrap();

    let mut state = BgState::new();
    state.now_secs = fake_now;
    state.startup_watermark = Some(T0 - 5_000);
    let (event_tx, _event_rx) = mpsc::channel(8);
    let (control_tx, _control_rx) = mpsc::channel(1);
    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    let task = tokio::spawn(run_background_task_with_state(
        state,
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
            replay_since: Some(T0 - 5_000),
        })
        .await
        .unwrap();
    cmd_tx
        .send(RelayCommand::SubscribeMembership)
        .await
        .unwrap();
    next_reqs_and_keepalive(&mut conn0, 2).await;

    for sub in [channel_sub_id(channel), MEMBERSHIP_NOTIF_SUB_ID.to_owned()] {
        conn0.ctl.send(MockCtl::Send(json!(["EOSE", sub]))).unwrap();
    }
    // The socket owner handles frames in order, so its Pong proves both EOSEs
    // were processed (and timestamped) at T0. It does NOT prove the Ping itself
    // was: the Pong is written while the Ping is being read, before the loop
    // stamps it.
    conn0.ctl.send(MockCtl::Ping).unwrap();
    loop {
        let frame = timeout(Duration::from_secs(5), conn0.frames.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(frame, MockFrame::Pong) {
            break;
        }
    }
    // The loop runs one `select!` arm to completion before the next, so a REQ
    // written for a command sent now proves the Ping's arm (including its
    // timestamp) is finished. Only then does the clock move.
    let throwaway = Uuid::new_v4();
    cmd_tx
        .send(RelayCommand::Subscribe {
            channel_id: throwaway,
            filter: test_channel_filter(),
            replay_since: Some(T0 - 5_000),
        })
        .await
        .unwrap();
    let barrier_req = next_req(&mut conn0).await;
    assert_eq!(barrier_req[1], channel_sub_id(throwaway));

    // Two hours pass with nothing from the relay; then the loss is noticed.
    // Longer than the silent-drop repair window, so a `live_until` stamped at
    // the wake time would put the cursor past a mention stored early in the
    // sleep.
    FAKE_NOW.store(T0 + 7_200, Ordering::SeqCst);
    if let Some(frame) = buffered {
        conn0.ctl.send(frame).unwrap();
    }
    conn0.ctl.send(loss).unwrap();
    // A 1012 close waits a random [0, 10s] before the first reconnect attempt.
    let mut conn1 = timeout(Duration::from_secs(20), conns.recv())
        .await
        .expect("socket owner reconnected")
        .unwrap();

    let mention_in_gap = T0 + 600;
    // The throwaway subscription is replayed too; only the two that reached
    // EOSE are checked.
    let mut pending: HashSet<String> =
        [channel_sub_id(channel), MEMBERSHIP_NOTIF_SUB_ID.to_owned()]
            .into_iter()
            .collect();
    while !pending.is_empty() {
        let req = next_req(&mut conn1).await;
        if !pending.remove(req[1].as_str().unwrap()) {
            continue;
        }
        let since = req[2]["since"].as_u64().unwrap();
        // Inbound frames are stamped from the monotonic clock, which keeps
        // running in real time here; allow a few seconds of test runtime.
        let expected = T0 - SILENT_DROP_REPAIR_WINDOW_SECS - SINCE_SKEW_SECS;
        assert!(
            (expected..=expected + 5).contains(&since),
            "{} must resume from the last inbound frame before the sleep: since={since}, expected~{expected}",
            req[1]
        );
        assert!(since <= mention_in_gap, "{} would skip the mention", req[1]);
    }

    cmd_tx.send(RelayCommand::Shutdown).await.unwrap();
    timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn resubscribing_after_unsubscribe_on_one_socket_ignores_the_old_reqs_eose() {
    let (mut client, mut server) = test_ws_pair().await;
    let mut state = BgState::new();
    let channel = Uuid::new_v4();
    let sub = channel_sub_id(channel);
    let subscribe = |replay_since| RelayCommand::Subscribe {
        channel_id: channel,
        filter: test_channel_filter(),
        replay_since: Some(replay_since),
    };

    // REQ #1 (recent floor), then the channel is removed and re-added with an
    // older floor, e.g. membership flapped. The wire ID is the same.
    assert!(
        execute_connected_command(&mut client, &mut state, "agent", subscribe(5_000_000)).await
    );
    assert_eq!(next_test_frame(&mut server).await[0], "REQ");
    assert!(
        execute_connected_command(
            &mut client,
            &mut state,
            "agent",
            RelayCommand::Unsubscribe {
                channel_id: channel
            },
        )
        .await
    );
    assert_eq!(next_test_frame(&mut server).await[0], "CLOSE");
    assert!(execute_connected_command(&mut client, &mut state, "agent", subscribe(1_000)).await);
    assert_eq!(next_test_frame(&mut server).await[0], "REQ");

    // The old REQ's EOSE is still in flight when it lands, then the socket
    // drops before the replacement has replayed history from 1_000.
    deliver_eose(&mut client, &mut state, &sub).await;
    lose_connection(&mut state).await;

    let since = first_replay_req(&mut state, &sub).await;
    assert!(
        since <= 1_000,
        "replay must keep the re-subscribe's older floor, got since={since}"
    );
}
