//! Capacity-wake boundaries and the independently reproduced R1 schedule.
use super::*;

// Reviewer-authored, exact-source comparison of recurring headroom at the socket owner.
// A small periodically refilled queue is empty for most of each five-second period.
async fn review_count_until(
    server: &mut WebSocketStream<tokio::net::TcpStream>,
    deadline: tokio::time::Instant,
) -> usize {
    let mut count = 0;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match socket_frame(server, remaining).await {
            None => break,
            Some(Message::Text(text)) => {
                let frame: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(frame[0], "REQ");
                count += 1;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    count
}

async fn review_barrier(server: &mut WebSocketStream<tokio::net::TcpStream>) -> usize {
    server.send(Message::Ping(vec![77].into())).await.unwrap();
    let mut count = 0;
    loop {
        match socket_frame(server, Duration::from_secs(2)).await.unwrap() {
            Message::Pong(payload) => {
                assert_eq!(payload.as_ref(), &[77]);
                return count;
            }
            Message::Text(text) => {
                let frame: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(frame[0], "REQ");
                count += 1;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[tokio::test]
async fn review_recurring_headroom_between_ticks_gets_an_attempt() {
    let (client, mut server) = test_ws_pair().await;
    let (tx, mut rx) = mpsc::channel(1);
    let (control_tx, _control_rx) = mpsc::channel(1);
    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    let start = tokio::time::Instant::now();
    let task = tokio::spawn(run_background_task(
        client,
        VecDeque::new(),
        tx,
        control_tx,
        cmd_rx,
        Keys::generate(),
        "ws://127.0.0.1:1".into(),
        "agent".into(),
        None,
    ));
    let ch = Uuid::new_v4();
    let sub = channel_sub_id(ch);
    cmd_tx
        .send(RelayCommand::Subscribe {
            channel_id: ch,
            filter: test_channel_filter(),
            replay_since: Some(1000),
        })
        .await
        .unwrap();
    let frame = socket_frame(&mut server, Duration::from_secs(2))
        .await
        .unwrap();
    let req: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
    assert_eq!(req[1], sub);
    let lost = fixture_event(ch, 1, 9);
    for event in [fixture_event(ch, 0, 9), lost.clone()] {
        server
            .send(Message::Text(
                json!(["EVENT", sub, event]).to_string().into(),
            ))
            .await
            .unwrap();
    }
    let mut attempts = review_barrier(&mut server).await;
    assert_eq!(rx.len(), 1);
    rx.recv().await.unwrap().unwrap();
    for round in 1..=4 {
        // Headroom until 1s before the tick; then only one live arrival, no new
        // overflow. The queue stays full across the tick and drains 0.5s later.
        attempts += review_count_until(
            &mut server,
            start + Duration::from_millis(round * 5000 - 1000),
        )
        .await;
        assert_eq!(rx.len(), 0);
        server
            .send(Message::Text(
                json!(["EVENT", sub, fixture_event(ch, 10 + round, 9)])
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        attempts += review_barrier(&mut server).await;
        assert_eq!(rx.len(), 1);
        attempts += review_count_until(
            &mut server,
            start + Duration::from_millis(round * 5000 + 500),
        )
        .await;
        rx.recv().await.unwrap().unwrap();
    }
    println!("REVIEW periodic consumer: 4 full-at-tick windows, empty >=3.5s each period, recovery_requests={attempts}");
    assert_eq!(
        attempts, 1,
        "recurring headroom must not strand the first attempt"
    );
    // Return the missing event using the actual requested stable subscription.
    server
        .send(Message::Text(
            json!(["EVENT", sub, lost]).to_string().into(),
        ))
        .await
        .unwrap();
    assert_eq!(review_barrier(&mut server).await, 0);
    assert_eq!(rx.recv().await.unwrap().unwrap().event.id, lost.id);
    let after = review_count_until(&mut server, start + Duration::from_millis(25_500)).await;
    assert_eq!(
        after, 0,
        "no timer churn or extra requests after successful write"
    );
    println!(
        "REVIEW continuous-headroom control: additional_requests={after}; missing event delivered"
    );
    cmd_tx.send(RelayCommand::Shutdown).await.unwrap();
    timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
}

/// Same real socket owner as production, with no control of its internal state.
struct Owner {
    server: WebSocketStream<tokio::net::TcpStream>,
    rx: mpsc::Receiver<Option<BuzzEvent>>,
    cmd: mpsc::Sender<RelayCommand>,
    task: tokio::task::JoinHandle<()>,
    ch: Uuid,
}

impl Owner {
    async fn new(capacity: usize) -> Self {
        let (client, server) = test_ws_pair().await;
        let (tx, rx) = mpsc::channel(capacity);
        let (control_tx, _control_rx) = mpsc::channel(1);
        let (cmd, cmd_rx) = mpsc::channel(8);
        let task = tokio::spawn(run_background_task(
            client,
            VecDeque::new(),
            tx,
            control_tx,
            cmd_rx,
            Keys::generate(),
            "ws://127.0.0.1:1".into(),
            "agent".into(),
            None,
        ));
        let mut owner = Self {
            server,
            rx,
            cmd,
            task,
            ch: Uuid::new_v4(),
        };
        owner.subscribe().await;
        owner
    }

    async fn subscribe(&mut self) {
        self.cmd
            .send(RelayCommand::Subscribe {
                channel_id: self.ch,
                filter: test_channel_filter(),
                replay_since: Some(1000),
            })
            .await
            .unwrap();
        let req = self.request(Duration::from_secs(2)).await;
        assert_eq!(req[1], channel_sub_id(self.ch));
    }

    async fn event(&mut self, n: u64) {
        self.server
            .send(Message::Text(
                json!([
                    "EVENT",
                    channel_sub_id(self.ch),
                    fixture_event(self.ch, n, 9)
                ])
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
    }

    async fn request(&mut self, duration: Duration) -> Value {
        let frame = socket_frame(&mut self.server, duration)
            .await
            .expect("recovery not woken");
        let req: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
        assert_eq!(req[0], "REQ");
        req
    }

    async fn shutdown(self) {
        self.cmd.send(RelayCommand::Shutdown).await.unwrap();
        timeout(Duration::from_secs(1), self.task)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn capacity_flapping_cannot_storm_or_delay_an_allowed_attempt() {
    let mut owner = Owner::new(1).await;
    owner.event(0).await;
    owner.event(1).await;
    assert_eq!(review_barrier(&mut owner.server).await, 0);
    owner.rx.recv().await.unwrap();
    let first = owner.request(Duration::from_millis(500)).await;
    let first_at = tokio::time::Instant::now();
    assert_eq!(first[2]["since"], 996);
    // New loss, then rapid full/empty transitions during the attempt cooldown.
    // No socket activity or capacity transition may reset or bypass that bound.
    for n in 1..=20 {
        owner.event(n * 2).await;
        owner.event(n * 2 + 1).await;
        assert_eq!(review_barrier(&mut owner.server).await, 0);
        owner.rx.recv().await.unwrap();
        assert!(socket_frame(&mut owner.server, Duration::from_millis(10))
            .await
            .is_none());
    }
    let req = owner.request(Duration::from_secs(6)).await;
    assert_eq!(req[1], channel_sub_id(owner.ch));
    // Arrival timestamps approximate send completion; leave tolerance for TCP.
    assert!(first_at.elapsed() >= Duration::from_millis(4_900));
    assert!(first_at.elapsed() < Duration::from_secs(6));
    assert_eq!(req[2]["since"], 998);
    assert!(socket_frame(&mut owner.server, Duration::from_millis(100))
        .await
        .is_none());
    owner.shutdown().await;
}

#[tokio::test]
async fn partial_capacity_wait_does_not_steal_live_slots_and_cancels_on_unsubscribe() {
    let mut owner = Owner::new(5).await; // odd capacity: threshold rounds UP to 3
    for n in 0..6 {
        owner.event(n).await;
    }
    assert_eq!(review_barrier(&mut owner.server).await, 0);
    owner.rx.recv().await.unwrap(); // partial reservation, insufficient for replay
    assert!(socket_frame(&mut owner.server, Duration::from_millis(30))
        .await
        .is_none());
    owner.event(6).await;
    assert_eq!(review_barrier(&mut owner.server).await, 0);
    assert_eq!(
        owner.rx.len(),
        5,
        "select must release partial permits BEFORE try_send"
    );
    for _ in 0..2 {
        owner.rx.recv().await.unwrap();
    }
    assert!(socket_frame(&mut owner.server, Duration::from_millis(30))
        .await
        .is_none());
    owner.rx.recv().await.unwrap(); // exactly three slots free, prompt capacity wake
    let req = owner.request(Duration::from_millis(500)).await;
    assert_eq!(req[2]["since"], 1000);
    assert_eq!(
        owner
            .rx
            .recv()
            .await
            .unwrap()
            .unwrap()
            .event
            .created_at
            .as_secs(),
        1004
    );
    assert_eq!(
        owner
            .rx
            .recv()
            .await
            .unwrap()
            .unwrap()
            .event
            .created_at
            .as_secs(),
        1006
    );

    // Wait out cooldown then create another pending loss and partial reservation.
    tokio::time::sleep(recovery::RECOVERY_INTERVAL).await;
    for n in 7..13 {
        owner.event(n).await;
    }
    assert_eq!(review_barrier(&mut owner.server).await, 0);
    owner.rx.recv().await.unwrap();
    assert!(socket_frame(&mut owner.server, Duration::from_millis(30))
        .await
        .is_none());
    owner
        .cmd
        .send(RelayCommand::Unsubscribe {
            channel_id: owner.ch,
        })
        .await
        .unwrap();
    let close = socket_frame(&mut owner.server, Duration::from_secs(1))
        .await
        .unwrap();
    let close: Value = serde_json::from_str(close.to_text().unwrap()).unwrap();
    assert_eq!(close[0], "CLOSE");
    while owner.rx.try_recv().is_ok() {}
    assert!(socket_frame(&mut owner.server, Duration::from_millis(100))
        .await
        .is_none());
    owner.subscribe().await;
    assert!(socket_frame(&mut owner.server, Duration::from_millis(100))
        .await
        .is_none());
    // A re-added intent can record and recover fresh loss immediately.
    for n in 20..26 {
        owner.event(n).await;
    }
    assert_eq!(review_barrier(&mut owner.server).await, 0);
    while owner.rx.try_recv().is_ok() {}
    let req = owner.request(Duration::from_millis(500)).await;
    assert_eq!(req[2]["since"], 1020);
    owner.shutdown().await;
}

#[tokio::test]
async fn shutdown_and_transport_loss_cancel_a_capacity_wait() {
    let mut owner = Owner::new(1).await;
    owner.event(0).await;
    owner.event(1).await;
    assert_eq!(review_barrier(&mut owner.server).await, 0);
    // Full queue cannot block commands or processing an actual socket close.
    owner.server.close(None).await.unwrap();
    owner.shutdown().await;
    let mut owner = Owner::new(1).await;
    owner.event(0).await;
    owner.event(1).await;
    assert_eq!(review_barrier(&mut owner.server).await, 0);
    owner.shutdown().await;
}

#[tokio::test]
async fn readiness_gate_ownership_and_attempt_deadlines_are_not_polling_ticks() {
    let (mut client, mut server) = test_ws_pair().await;
    let mut state = BgState::new();
    let ch = Uuid::new_v4();
    seed_test_subscription(&mut state, ch);
    let (tx, mut rx) = mpsc::channel(4);
    assert!(recovery::ready_at(&mut state).is_none());
    state.channel_dropped_since.insert(ch, 700);
    state
        .rate_limited_pending
        .insert(ch, tokio::time::Instant::now());
    assert!(recovery::ready_at(&mut state).is_none());
    state.rate_limited_pending.clear();
    state.resubscribe_retry.insert(ch);
    assert!(recovery::ready_at(&mut state).is_none());
    state.resubscribe_retry.clear();
    state.rate_limit_gate = Some(tokio::time::Instant::now() + Duration::from_millis(80));
    let at = recovery::ready_at(&mut state);
    assert_eq!(at, state.rate_limit_gate);
    assert!(timeout(Duration::from_millis(20), recovery::ready(&tx, at))
        .await
        .is_err());
    timeout(Duration::from_millis(200), recovery::ready(&tx, at))
        .await
        .unwrap();
    recovery::recover_one(&mut client, &mut state, &tx, "agent").await;
    next_test_frame(&mut server).await;
    state.channel_dropped_since.insert(ch, 800);
    let at = recovery::ready_at(&mut state).unwrap();
    let remaining = at.saturating_duration_since(tokio::time::Instant::now());
    assert!(remaining > Duration::from_millis(4900));
    // Neither new loss nor an intervening gate shorter than cooldown delays it.
    state.rate_limit_gate = Some(tokio::time::Instant::now() + Duration::from_millis(10));
    assert_eq!(recovery::ready_at(&mut state), Some(at));
    state.rate_limit_gate = Some(at + Duration::from_secs(1));
    assert_eq!(recovery::ready_at(&mut state), state.rate_limit_gate);
    advance_clock(Duration::from_secs(6)).await;
    // Cancellation releases partial permits. No hidden reservation survives it.
    for _ in 0..3 {
        tx.try_send(None).unwrap();
    }
    assert!(timeout(
        Duration::from_millis(20),
        recovery::ready(&tx, recovery::ready_at(&mut state))
    )
    .await
    .is_err());
    assert_eq!(tx.capacity(), 1);
    rx.recv().await.unwrap();
    timeout(
        Duration::from_millis(100),
        recovery::ready(&tx, recovery::ready_at(&mut state)),
    )
    .await
    .unwrap();
    assert_eq!(tx.capacity(), 2);
    drop(rx);
    assert!(timeout(
        Duration::from_millis(20),
        recovery::ready(&tx, recovery::ready_at(&mut state))
    )
    .await
    .is_err());
}

#[tokio::test]
async fn readiness_uses_channel_wakes_without_idle_churn_or_lost_capacity() {
    use std::future::Future;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::task::{Context, Wake, Waker};
    #[derive(Default)]
    struct Wakes(AtomicUsize);
    impl Wake for Wakes {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let wakes = Arc::new(Wakes::default());
    let waker = Waker::from(wakes.clone());
    let mut cx = Context::from_waker(&waker);
    let (tx, mut rx) = mpsc::channel(4);
    for _ in 0..4 {
        tx.try_send(None).unwrap();
    }
    let mut idle = Box::pin(recovery::ready(&tx, None));
    assert!(idle.as_mut().poll(&mut cx).is_pending());
    rx.recv().await.unwrap();
    rx.recv().await.unwrap();
    assert_eq!(
        wakes.0.load(Ordering::SeqCst),
        0,
        "no loss: no capacity subscription"
    );
    drop(idle);
    // Capacity freed before registration cannot be lost; ready on first poll.
    let now = Some(tokio::time::Instant::now());
    let mut ready = Box::pin(recovery::ready(&tx, now));
    assert!(ready.as_mut().poll(&mut cx).is_ready());
    assert_eq!(tx.capacity(), 2, "successful readiness returns all permits");
    drop(ready);
    for _ in 0..2 {
        tx.try_send(None).unwrap();
    }
    let mut ready = Box::pin(recovery::ready(&tx, now));
    assert!(ready.as_mut().poll(&mut cx).is_pending());
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    rx.recv().await.unwrap();
    assert_eq!(
        wakes.0.load(Ordering::SeqCst),
        0,
        "below threshold: do not wake"
    );
    rx.recv().await.unwrap();
    assert_eq!(
        wakes.0.load(Ordering::SeqCst),
        1,
        "threshold: channel wakes its waiter"
    );
    assert!(ready.as_mut().poll(&mut cx).is_ready());
    assert_eq!(tx.capacity(), 2);
    drop(ready);
    drop(rx);
    let mut closed = Box::pin(recovery::ready(&tx, now));
    let before = wakes.0.load(Ordering::SeqCst);
    assert!(closed.as_mut().poll(&mut cx).is_pending());
    assert_eq!(
        wakes.0.load(Ordering::SeqCst),
        before,
        "closed: no self-wake loop"
    );
}
