use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{backoff_delay, run, AudioConnection, ReconnectTarget, RECONNECT_WINDOW};
use crate::app_state::{build_app_state, AppState};
use crate::huddle::relay_api::AudioRelayConnectError;
use crate::huddle::state::{AudioLink, HuddlePhase};

const HUDDLE: &str = "huddle-a";

fn live_huddle_state() -> Arc<AppState> {
    let state = build_app_state();
    {
        let mut hs = state.huddle().expect("huddle lock");
        hs.phase = HuddlePhase::Active;
        hs.ephemeral_channel_id = Some(HUDDLE.into());
        hs.parent_channel_id = Some("parent".into());
        hs.begin_huddle_lifetime();
        hs.audio_ws_cancel = Some(CancellationToken::new());
    }
    Arc::new(state)
}

fn target(state: &AppState) -> ReconnectTarget {
    ReconnectTarget::capture(&state.huddle().unwrap(), HUDDLE, Some("parent"))
}

fn connection() -> AudioConnection {
    (CancellationToken::new(), tokio::sync::mpsc::channel(1).0)
}

fn refused(code: &str) -> AudioRelayConnectError {
    AudioRelayConnectError::from_relay_payload(&serde_json::json!({
        "code": code,
        "message": "refused",
    }))
}

fn audio_link(state: &AppState) -> AudioLink {
    state.huddle().expect("huddle lock").audio_link.clone()
}

/// Dial that fails until `succeed_after` of virtual time has passed.
fn flaky_dial(
    dials: Arc<AtomicU32>,
    started: Instant,
    succeed_after: Option<Duration>,
) -> impl FnMut(ReconnectTarget) -> std::future::Ready<Result<AudioConnection, AudioRelayConnectError>>
{
    move |_| {
        dials.fetch_add(1, Ordering::SeqCst);
        let ok = succeed_after.is_some_and(|after| started.elapsed() >= after);
        std::future::ready(if ok {
            Ok(connection())
        } else {
            Err("connection refused".into())
        })
    }
}

#[tokio::test(start_paused = true)]
async fn recovers_after_a_thirty_five_second_outage_without_leaving() {
    let state = live_huddle_state();
    let dials = Arc::new(AtomicU32::new(0));
    let started = Instant::now();

    run(
        &state,
        target(&state),
        flaky_dial(Arc::clone(&dials), started, Some(Duration::from_secs(35))),
    )
    .await;

    assert!(started.elapsed() >= Duration::from_secs(35));
    assert!(dials.load(Ordering::SeqCst) > 1, "must have retried");
    let hs = state.huddle().expect("huddle lock");
    assert_eq!(hs.audio_link, AudioLink::Live);
    assert_eq!(hs.phase, HuddlePhase::Active, "phase never leaves Active");
    assert!(hs.audio_relay_pcm_tx.is_some(), "new sender installed");
    assert!(hs
        .audio_ws_cancel
        .as_ref()
        .is_some_and(|c| !c.is_cancelled()));
}

#[tokio::test(start_paused = true)]
async fn draining_refusal_redials_promptly_instead_of_backing_off() {
    let state = live_huddle_state();
    let dial_times = Arc::new(std::sync::Mutex::new(Vec::<Instant>::new()));
    let times = Arc::clone(&dial_times);

    run(&state, target(&state), move |_| {
        let mut t = times.lock().unwrap();
        t.push(Instant::now());
        let n = t.len();
        std::future::ready(match n {
            // Two ordinary refusals grow the backoff; the drain hint resets it.
            1 | 2 => Err("connection refused".into()),
            3 => Err(refused("huddle_relay_draining")),
            _ => Ok(connection()),
        })
    })
    .await;

    let t = dial_times.lock().unwrap();
    assert_eq!(t.len(), 4);
    let ordinary_gap = t[2] - t[1];
    let draining_gap = t[3] - t[2];
    assert!(
        draining_gap <= Duration::from_millis(250),
        "{draining_gap:?}"
    );
    assert!(
        draining_gap < ordinary_gap,
        "{draining_gap:?} vs {ordinary_gap:?}"
    );
    assert_eq!(audio_link(&state), AudioLink::Live);
}

#[tokio::test(start_paused = true)]
async fn draining_is_visible_in_state_while_waiting() {
    let state = live_huddle_state();
    let seen = Arc::new(std::sync::Mutex::new(Vec::<AudioLink>::new()));
    let observer = Arc::clone(&seen);
    let observed_state = Arc::clone(&state);

    run(&state, target(&state), move |_| {
        observer.lock().unwrap().push(audio_link(&observed_state));
        let n = observer.lock().unwrap().len();
        std::future::ready(match n {
            1 => Err(refused("huddle_relay_draining")),
            _ => Ok(connection()),
        })
    })
    .await;

    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.as_slice(),
        [
            AudioLink::Reconnecting {
                attempt: 0,
                draining: false
            },
            AudioLink::Reconnecting {
                attempt: 1,
                draining: true
            },
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn leave_during_backoff_stops_the_loop_and_never_installs_a_socket() {
    let state = live_huddle_state();
    let dials = Arc::new(AtomicU32::new(0));
    let loop_state = Arc::clone(&state);
    let loop_dials = Arc::clone(&dials);

    let loop_task = tokio::spawn(async move {
        run(&loop_state, target(&loop_state), move |_| {
            loop_dials.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Err::<AudioConnection, _>("connection refused".into()))
        })
        .await;
    });
    tokio::task::yield_now().await;
    assert!(matches!(audio_link(&state), AudioLink::Reconnecting { .. }));

    // Intentional leave while the loop sleeps between dials.
    state
        .huddle()
        .expect("huddle lock")
        .reset_preserving_generation();

    tokio::time::timeout(Duration::from_millis(1), loop_task)
        .await
        .expect("loop must exit on leave, not run out the window")
        .expect("loop task");
    let dials_at_exit = dials.load(Ordering::SeqCst);
    assert!(dials_at_exit <= 2, "{dials_at_exit} dials after leave");
    let hs = state.huddle().expect("huddle lock");
    assert_eq!(hs.phase, HuddlePhase::Idle);
    assert_eq!(hs.audio_link, AudioLink::Live, "idle state is not Lost");
    assert!(hs.audio_ws_cancel.is_none());
}

#[tokio::test(start_paused = true)]
async fn replacement_huddle_during_backoff_is_left_untouched() {
    let state = live_huddle_state();
    let loop_state = Arc::clone(&state);
    let returned = Arc::new(std::sync::Mutex::new(Vec::<CancellationToken>::new()));
    let returned_for_dial = Arc::clone(&returned);

    let loop_task = tokio::spawn(async move {
        run(&loop_state, target(&loop_state), move |_| {
            let mut r = returned_for_dial.lock().unwrap();
            std::future::ready(if r.is_empty() {
                // First dial fails so the loop sleeps; the huddle is swapped
                // underneath it, and the next dial "succeeds" for the old one.
                r.push(CancellationToken::new());
                Err("connection refused".into())
            } else {
                let (cancel, tx) = connection();
                r.push(cancel.clone());
                Ok((cancel, tx))
            })
        })
        .await;
    });
    tokio::task::yield_now().await;

    let replacement_cancel = CancellationToken::new();
    {
        let mut hs = state.huddle().expect("huddle lock");
        hs.reset_preserving_generation();
        hs.phase = HuddlePhase::Active;
        hs.ephemeral_channel_id = Some("huddle-b".into());
        hs.begin_huddle_lifetime();
        hs.audio_ws_cancel = Some(replacement_cancel.clone());
    }

    tokio::time::timeout(Duration::from_millis(1), loop_task)
        .await
        .expect("loop must exit when the huddle is replaced")
        .expect("loop task");
    let hs = state.huddle().expect("huddle lock");
    assert_eq!(hs.ephemeral_channel_id.as_deref(), Some("huddle-b"));
    assert!(
        !replacement_cancel.is_cancelled(),
        "new huddle's socket untouched"
    );
    assert_eq!(hs.audio_link, AudioLink::Live);
    let returned = returned.lock().unwrap();
    // Any socket opened for the stale huddle is cancelled, not installed.
    for stale in returned.iter().skip(1) {
        assert!(stale.is_cancelled());
    }
}

#[tokio::test(start_paused = true)]
async fn leave_while_a_dial_is_in_flight_cancels_the_fresh_socket() {
    let state = live_huddle_state();
    let dial_state = Arc::clone(&state);
    let fresh = CancellationToken::new();
    let fresh_for_dial = fresh.clone();

    run(&state, target(&state), move |_| {
        // The user leaves while the dial is on the wire; the dial still wins
        // a socket for the huddle that no longer exists.
        dial_state
            .huddle()
            .expect("huddle lock")
            .reset_preserving_generation();
        std::future::ready(Ok((
            fresh_for_dial.clone(),
            tokio::sync::mpsc::channel(1).0,
        )))
    })
    .await;

    assert!(fresh.is_cancelled(), "stale socket must be cancelled");
    let hs = state.huddle().expect("huddle lock");
    assert_eq!(hs.phase, HuddlePhase::Idle);
    assert!(
        hs.audio_ws_cancel.is_none(),
        "nothing installed on an idle huddle"
    );
    assert!(hs.audio_relay_pcm_tx.is_none());
}

#[tokio::test(start_paused = true)]
async fn replacement_pipeline_dying_before_install_is_redialed_not_installed() {
    let state = live_huddle_state();
    let installed = Arc::new(std::sync::Mutex::new(Vec::<CancellationToken>::new()));
    let handed_out = Arc::clone(&installed);

    run(&state, target(&state), move |_| {
        let (cancel, pcm_tx) = connection();
        let mut h = handed_out.lock().unwrap();
        if h.is_empty() {
            // Auth and join succeed, then the spawned pipeline exits before
            // the loop installs it: the pipeline cancels its own token and
            // its disconnect callback is refused by `claim_reconnect`.
            cancel.cancel();
        }
        h.push(cancel.clone());
        std::future::ready(Ok((cancel, pcm_tx)))
    })
    .await;

    let installed = installed.lock().unwrap();
    assert_eq!(installed.len(), 2, "dead replacement must be redialed");
    let hs = state.huddle().expect("huddle lock");
    assert_eq!(hs.audio_link, AudioLink::Live);
    let live = hs.audio_ws_cancel.as_ref().expect("socket installed");
    assert!(!live.is_cancelled(), "Live must never hold a dead pipeline");
    assert!(
        hs.audio_relay_pcm_tx.is_some(),
        "sender belongs to the live pipeline"
    );
}

#[tokio::test(start_paused = true)]
async fn duplicate_disconnect_signals_do_not_start_a_second_loop() {
    let state = live_huddle_state();
    let dials = Arc::new(AtomicU32::new(0));
    let started = Instant::now();

    let first_state = Arc::clone(&state);
    let first_dials = Arc::clone(&dials);
    let first = tokio::spawn(async move {
        run(
            &first_state,
            target(&first_state),
            flaky_dial(first_dials, started, Some(Duration::from_secs(3))),
        )
        .await;
    });
    tokio::task::yield_now().await;

    let second_dials = Arc::new(AtomicU32::new(0));
    run(
        &state,
        target(&state),
        flaky_dial(Arc::clone(&second_dials), started, None),
    )
    .await;
    assert_eq!(
        second_dials.load(Ordering::SeqCst),
        0,
        "second loop must not dial"
    );

    first.await.expect("first loop");
    assert_eq!(audio_link(&state), AudioLink::Live);
}

#[tokio::test(start_paused = true)]
async fn exhausting_the_window_marks_audio_lost_and_ends_the_loop() {
    let state = live_huddle_state();
    let dials = Arc::new(AtomicU32::new(0));
    let started = Instant::now();

    tokio::time::timeout(
        RECONNECT_WINDOW * 2,
        run(
            &state,
            target(&state),
            flaky_dial(Arc::clone(&dials), started, None),
        ),
    )
    .await
    .expect("loop must terminate (no task leak)");

    assert_eq!(
        started.elapsed(),
        RECONNECT_WINDOW,
        "backoff must not overshoot"
    );
    let hs = state.huddle().expect("huddle lock");
    assert_eq!(hs.audio_link, AudioLink::Lost);
    assert_eq!(hs.phase, HuddlePhase::Active, "still joined; user decides");
    assert!(hs.audio_relay_pcm_tx.is_none());
    let n = dials.load(Ordering::SeqCst);
    assert!((12..=300).contains(&n), "{n} dials in the window");
}

#[tokio::test(start_paused = true)]
async fn not_live_huddle_is_ignored() {
    let state = Arc::new(build_app_state());
    let dials = Arc::new(AtomicU32::new(0));
    run(
        &state,
        target(&state),
        flaky_dial(Arc::clone(&dials), Instant::now(), None),
    )
    .await;
    assert_eq!(dials.load(Ordering::SeqCst), 0);
    assert_eq!(audio_link(&state), AudioLink::Live);
}

#[test]
fn backoff_grows_caps_and_jitters_within_half_to_full() {
    assert_eq!(backoff_delay(1, 1.0), Duration::from_millis(250));
    assert_eq!(backoff_delay(2, 1.0), Duration::from_millis(500));
    assert_eq!(backoff_delay(6, 1.0), Duration::from_secs(5));
    assert_eq!(
        backoff_delay(40, 1.0),
        Duration::from_secs(5),
        "no overflow"
    );
    assert_eq!(backoff_delay(1, 0.0), Duration::from_millis(125));
    assert_eq!(backoff_delay(1, 7.0), Duration::from_millis(250), "clamped");
}

#[test]
fn audio_link_serializes_as_tagged_status() {
    let live = serde_json::to_value(AudioLink::Live).unwrap();
    assert_eq!(live, serde_json::json!({ "status": "live" }));
    let reconnecting = serde_json::to_value(AudioLink::Reconnecting {
        attempt: 2,
        draining: true,
    })
    .unwrap();
    assert_eq!(
        reconnecting,
        serde_json::json!({ "status": "reconnecting", "attempt": 2, "draining": true })
    );
}

#[tokio::test(start_paused = true)]
async fn hung_dials_stop_at_the_recovery_deadline() {
    let state = live_huddle_state();
    let started = Instant::now();
    let dials = Arc::new(AtomicU32::new(0));
    let calls = Arc::clone(&dials);
    tokio::time::timeout(
        RECONNECT_WINDOW + Duration::from_secs(1),
        run(&state, target(&state), move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<Result<AudioConnection, AudioRelayConnectError>>()
        }),
    )
    .await
    .expect("hung upgrade must be bounded");
    assert_eq!(started.elapsed(), RECONNECT_WINDOW);
    assert_eq!(audio_link(&state), AudioLink::Lost);
    assert!(
        dials.load(Ordering::SeqCst) > 1,
        "per-dial cap must allow retries"
    );
}

#[tokio::test(start_paused = true)]
async fn leave_interrupts_a_hung_dial_immediately() {
    let state = live_huddle_state();
    let recovery = run(&state, target(&state), |_| {
        std::future::pending::<Result<AudioConnection, AudioRelayConnectError>>()
    });
    tokio::pin!(recovery);
    tokio::select! {
        _ = &mut recovery => panic!("dial should be pending"),
        _ = tokio::task::yield_now() => {},
    }
    let leaving = Instant::now();
    state.huddle().unwrap().reset_preserving_generation();
    tokio::time::timeout(Duration::from_millis(1), recovery)
        .await
        .expect("leave must wake pending recovery");
    assert_eq!(leaving.elapsed(), Duration::ZERO);
    assert_eq!(state.huddle().unwrap().phase, HuddlePhase::Idle);
}

#[tokio::test(start_paused = true)]
async fn stale_disconnect_cannot_claim_a_replacement_on_the_same_channel() {
    let state = live_huddle_state();
    let stale = target(&state);
    let replacement = CancellationToken::new();
    {
        let mut hs = state.huddle().unwrap();
        hs.begin_huddle_lifetime();
        hs.audio_ws_cancel = Some(replacement.clone());
    }
    run(&state, stale, |_| {
        panic!("stale callback must never dial");
        #[allow(unreachable_code)]
        std::future::ready(Ok(connection()))
    })
    .await;
    assert_eq!(audio_link(&state), AudioLink::Live);
    assert!(!replacement.is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn disconnect_identity_checks_channel_and_generation_even_with_a_live_token() {
    let state = live_huddle_state();
    for wrong_channel in [false, true] {
        let mut stale = target(&state);
        if wrong_channel {
            stale.ephemeral_channel_id = "other-channel".into();
        } else {
            stale.huddle_generation = stale.huddle_generation.wrapping_sub(1);
        }
        run(&state, stale, |_| {
            panic!("mismatched callback must not dial");
            #[allow(unreachable_code)]
            std::future::ready(Ok(connection()))
        })
        .await;
        assert_eq!(audio_link(&state), AudioLink::Live);
    }
}

#[test]
fn huddle_lifetime_cancellation_tracks_begin_reset_not_transcription() {
    let state = live_huddle_state();
    let first = target(&state);
    {
        let mut hs = state.huddle().unwrap();
        hs.invalidate_transcription_pipeline();
        assert!(
            !first.lifetime.is_cancelled(),
            "transcription is not the huddle lifetime"
        );
        hs.begin_huddle_lifetime();
    }
    assert!(
        first.lifetime.is_cancelled(),
        "new lifetime must cancel pending old work"
    );
    let second = target(&state);
    assert!(!second.lifetime.is_cancelled());
    state.huddle().unwrap().reset_preserving_generation();
    assert!(
        second.lifetime.is_cancelled(),
        "reset must wake pending work"
    );
}
