//! Real Redis counters through the production admission entrypoints. Tests use
//! disposable identities and expiring keys, never FLUSHDB or a shared identity.

use super::*;
use axum::extract::ws::Message as WsMessage;
use buzz_auth::{AuthService, RateLimitConfig};
use buzz_core::CommunityId;
use buzz_pubsub::rate_limiter::RedisRateLimiter;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use nostr::{EventBuilder, Keys, Kind, Tag};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::connection::tests::{
    authenticated_state, read_frame, test_conn_in, test_conn_with_auth, TestConn,
};

/// Limits for a test. Agent budgets are per (owner, channel) and per owner.
struct Limits {
    messages: u64,
    agent_channel: u64,
    agent_owner: u64,
    ws: u64,
    api_calls: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            messages: 60,
            agent_channel: 120,
            agent_owner: 600,
            ws: 100,
            api_calls: 300,
        }
    }
}

async fn state_with_limits(limits: Limits) -> AppState {
    with_limits((*crate::state::tests::test_state().await).clone(), limits)
}

/// `state` counting against the isolated test Redis under `limits`.
fn with_limits(mut state: AppState, limits: Limits) -> AppState {
    let url = std::env::var("BUZZ_TEST_REDIS_URL").expect("explicit isolated BUZZ_TEST_REDIS_URL");
    let mut auth = state.auth.config().clone();
    auth.rate_limits = RateLimitConfig {
        human_messages_per_min: limits.messages,
        agent_channel_events_per_min: limits.agent_channel,
        agent_owner_events_per_min: limits.agent_owner,
        human_ws_events_per_sec: limits.ws,
        human_api_calls_per_min: limits.api_calls,
        ..RateLimitConfig::default()
    };
    state.auth = Arc::new(AuthService::new(auth));
    state.redis_pool = deadpool_redis::Config::from_url(url)
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .unwrap();
    state.admission_rate_limiter = Arc::new(RedisRateLimiter::new(state.redis_pool.clone()));
    state
}

fn connection() -> (Arc<ConnectionState>, mpsc::Receiver<WsMessage>) {
    test_conn_with_auth(authenticated_state())
}

/// A socket for `agent` in `community`, admitted under `owner` and registered
/// the way AUTH registers it, so admission reads the owner from the same
/// record that owner revocation uses.
fn agent_socket(
    state: &AppState,
    community: CommunityId,
    agent: nostr::PublicKey,
    owner: nostr::PublicKey,
) -> (Arc<ConnectionState>, mpsc::Receiver<WsMessage>) {
    let AuthState::Authenticated(mut auth) = authenticated_state() else {
        unreachable!()
    };
    auth.pubkey = agent;
    let TestConn { conn, send_rx, .. } =
        test_conn_in(AuthState::Authenticated(auth), None, community);
    state.conn_manager.register(
        conn.conn_id,
        conn.send_tx.clone(),
        conn.ctrl_tx.clone(),
        conn.terminal_ctrl_tx.clone(),
        None,
        conn.cancel.clone(),
        community,
        conn.backpressure_count.clone(),
        conn.subscriptions.clone(),
        conn.grace_limit,
        conn.community_control.clone(),
    );
    state
        .conn_manager
        .set_admitted_owner(conn.conn_id, owner.to_bytes());
    (conn, send_rx)
}

/// A socket for a fresh agent key of `owner` in a fresh community.
fn agent_connection(
    state: &AppState,
    owner: nostr::PublicKey,
) -> (Arc<ConnectionState>, mpsc::Receiver<WsMessage>) {
    agent_socket(
        state,
        CommunityId::from_uuid(Uuid::new_v4()),
        Keys::generate().public_key(),
        owner,
    )
}

fn event(kind: u16) -> ClientMessage {
    channel_event(kind, None)
}

/// An EVENT of `kind`, tagged `h` with `channel` when given.
fn channel_event(kind: u16, channel: Option<Uuid>) -> ClientMessage {
    let mut builder = EventBuilder::new(Kind::from(kind), "");
    if let Some(channel) = channel {
        builder = builder.tag(Tag::parse(["h", &channel.to_string()]).unwrap());
    }
    let event = builder.sign_with_keys(&Keys::generate()).unwrap();
    ClientMessage::parse(&serde_json::json!(["EVENT", event]).to_string()).unwrap()
}

fn pubkey(conn: &ConnectionState) -> nostr::PublicKey {
    let AuthState::Authenticated(auth) = conn.auth_state_snapshot() else {
        unreachable!()
    };
    auth.pubkey
}

async fn get(state: &AppState, key: String) -> Option<u64> {
    let mut redis = state.redis_pool.get().await.unwrap();
    redis::cmd("GET")
        .arg(key)
        .query_async(&mut *redis)
        .await
        .unwrap()
}

/// Seconds until `key` expires.
async fn ttl(state: &AppState, key: &str) -> i64 {
    let mut redis = state.redis_pool.get().await.unwrap();
    redis::cmd("TTL")
        .arg(key)
        .query_async(&mut *redis)
        .await
        .unwrap()
}

async fn counter(state: &AppState, conn: &ConnectionState, bucket: LimitType) -> Option<u64> {
    get(
        state,
        buzz_auth::rate_limit::rate_limit_key(&conn.tenant, &pubkey(conn), &bucket),
    )
    .await
}

/// Events `owner`'s agents have been admitted in `channel` this window.
async fn channel_spent(
    state: &AppState,
    conn: &ConnectionState,
    owner: &nostr::PublicKey,
    channel: Option<Uuid>,
) -> Option<u64> {
    get(
        state,
        buzz_auth::agent_channel_budget_key(&conn.tenant, owner, channel),
    )
    .await
}

/// Events `owner`'s agents have been admitted across all channels.
async fn owner_spent(
    state: &AppState,
    conn: &ConnectionState,
    owner: &nostr::PublicKey,
) -> Option<u64> {
    get(
        state,
        buzz_auth::agent_owner_budget_key(&conn.tenant, owner),
    )
    .await
}

/// Asserts each `(transport, reason, bucket, count)` rejection counter. One
/// snapshot covers them all: taking a snapshot resets the recorder.
fn assert_metrics(recorder: &DebuggingRecorder, expected: &[(&str, &str, &str, u64)]) {
    let snapshot = recorder.snapshotter().snapshot().into_vec();
    for &(transport, reason, bucket, count) in expected {
        let labels = [
            ("transport", transport),
            ("reason", reason),
            ("bucket", bucket),
        ];
        let actual = snapshot.iter().find_map(|(key, _, _, value)| {
            (key.key().name() == "buzz_admission_rejections_total"
                && labels
                    .iter()
                    .all(|(k, v)| key.key().labels().any(|l| l.key() == *k && l.value() == *v)))
            .then(|| match value {
                DebugValue::Counter(value) => *value,
                _ => panic!("expected counter"),
            })
        });
        assert_eq!(actual, Some(count), "{transport}/{reason}/{bucket}");
    }
}

fn assert_metric(
    recorder: &DebuggingRecorder,
    transport: &str,
    reason: &str,
    bucket: &str,
    count: u64,
) {
    assert_metrics(recorder, &[(transport, reason, bucket, count)]);
}

/// Parses the exact retry grammar clients anchor on and returns its seconds.
fn retry_seconds(reason: &str) -> u64 {
    reason
        .strip_prefix("rate-limited: quota exceeded; retry in ")
        .unwrap()
        .strip_suffix('s')
        .unwrap()
        .parse()
        .unwrap()
}

fn assert_quota_frame(frame: &serde_json::Value, msg: &ClientMessage, max_retry: u64) {
    let reason = match msg {
        ClientMessage::Event(event) => {
            assert_eq!(frame[0], "OK");
            assert_eq!(frame[1], event.id.to_hex());
            assert_eq!(frame[2], false);
            frame[3].as_str().unwrap()
        }
        ClientMessage::Req { sub_id, .. } | ClientMessage::Count { sub_id, .. } => {
            assert_eq!(frame[0], "CLOSED");
            assert_eq!(frame[1], *sub_id);
            frame[2].as_str().unwrap()
        }
        _ => panic!("unexpected frame"),
    };
    // Exact grammar: clients including buzz-app anchor both ends of this hint.
    let seconds = retry_seconds(reason);
    assert!((1..=max_retry).contains(&seconds), "{reason}");
}

/// Submits `msg` and, when refused, checks the refusal is a quota frame.
async fn admit(
    msg: &ClientMessage,
    conn: &ConnectionState,
    rx: &mut mpsc::Receiver<WsMessage>,
    state: &AppState,
) -> bool {
    let admitted = enforce_ws_admission(msg, conn, state).await;
    if !admitted {
        assert_quota_frame(&read_frame(rx), msg, 60);
    }
    admitted
}

#[tokio::test]
#[ignore = "requires explicit isolated BUZZ_TEST_REDIS_URL"]
async fn ephemeral_events_leave_human_and_agent_message_budgets_untouched() {
    let state = state_with_limits(Limits {
        messages: 2,
        agent_channel: 3,
        ..Limits::default()
    })
    .await;
    let recorder = DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    // Both range boundaries, typing, presence, and observer frames. Adjacent
    // non-ephemeral kinds below must still consume the message allowance.
    let ephemeral_kinds = [20000, 20001, 20002, 24200, 29999];
    for stored_kind in [0, 1, 9, 19999, 30000, 40002] {
        let (conn, mut rx) = connection();
        for kind in ephemeral_kinds {
            assert!(enforce_ws_admission(&event(kind), &conn, &state).await);
        }
        assert_eq!(counter(&state, &conn, LimitType::Messages).await, None);
        assert_eq!(counter(&state, &conn, LimitType::WsEvents).await, Some(5));
        for expected in 1..=2 {
            assert!(enforce_ws_admission(&event(stored_kind), &conn, &state).await);
            assert_eq!(
                counter(&state, &conn, LimitType::Messages).await,
                Some(expected)
            );
        }
        let rejected = event(stored_kind);
        assert!(!enforce_ws_admission(&rejected, &conn, &state).await);
        assert_quota_frame(&read_frame(&mut rx), &rejected, 60);
        for kind in ephemeral_kinds {
            assert!(enforce_ws_admission(&event(kind), &conn, &state).await);
        }
        assert_eq!(counter(&state, &conn, LimitType::Messages).await, Some(3));
        assert!(rx.try_recv().is_err());

        // An agent spends its owner's budgets instead of a per-key one, and
        // ephemeral kinds leave those untouched too.
        let owner = Keys::generate().public_key();
        let (conn, mut rx) = agent_connection(&state, owner);
        for kind in ephemeral_kinds {
            assert!(enforce_ws_admission(&event(kind), &conn, &state).await);
        }
        assert_eq!(owner_spent(&state, &conn, &owner).await, None);
        assert_eq!(counter(&state, &conn, LimitType::WsEvents).await, Some(5));
        for expected in 1..=3 {
            assert!(enforce_ws_admission(&event(stored_kind), &conn, &state).await);
            assert_eq!(
                channel_spent(&state, &conn, &owner, None).await,
                Some(expected)
            );
        }
        let rejected = event(stored_kind);
        assert!(!enforce_ws_admission(&rejected, &conn, &state).await);
        assert_quota_frame(&read_frame(&mut rx), &rejected, 60);
        for kind in ephemeral_kinds {
            assert!(enforce_ws_admission(&event(kind), &conn, &state).await);
        }
        assert_eq!(channel_spent(&state, &conn, &owner, None).await, Some(3));
        assert_eq!(owner_spent(&state, &conn, &owner).await, Some(3));
        assert_eq!(counter(&state, &conn, LimitType::Messages).await, None);
        assert!(rx.try_recv().is_err());
    }
    assert_metrics(
        &recorder,
        &[
            ("websocket", "quota", "messages", 6),
            ("websocket", "quota", "agent_channel_events", 6),
        ],
    );
}

/// The owner/channel budget is one budget for every agent key of the owner:
/// a second key, a fresh key, and a reconnect of the first key all spend it,
/// and none gets capacity of its own.
#[tokio::test]
#[ignore = "requires explicit isolated BUZZ_TEST_REDIS_URL"]
async fn every_agent_key_of_an_owner_shares_its_channel_budget() {
    let state = state_with_limits(Limits {
        agent_channel: 3,
        ..Limits::default()
    })
    .await;
    let recorder = DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let community = CommunityId::from_uuid(Uuid::new_v4());
    let owner = Keys::generate().public_key();
    let channel = Some(Uuid::new_v4());
    let first_key = Keys::generate().public_key();
    let (first, mut first_rx) = agent_socket(&state, community, first_key, owner);
    let (second, mut second_rx) =
        agent_socket(&state, community, Keys::generate().public_key(), owner);

    // Quota boundary: the third event is admitted, the fourth is not.
    assert!(admit(&channel_event(9, channel), &first, &mut first_rx, &state).await);
    assert!(admit(&channel_event(9, channel), &second, &mut second_rx, &state).await);
    assert!(admit(&channel_event(9, channel), &first, &mut first_rx, &state).await);
    assert!(!admit(&channel_event(9, channel), &second, &mut second_rx, &state).await);

    let (fresh_key, mut fresh_rx) =
        agent_socket(&state, community, Keys::generate().public_key(), owner);
    assert!(
        !admit(
            &channel_event(9, channel),
            &fresh_key,
            &mut fresh_rx,
            &state
        )
        .await
    );
    let (reconnect, mut reconnect_rx) = agent_socket(&state, community, first_key, owner);
    assert!(
        !admit(
            &channel_event(9, channel),
            &reconnect,
            &mut reconnect_rx,
            &state
        )
        .await
    );

    // Refusals spent nothing, and no per-key message budget was used.
    assert_eq!(
        channel_spent(&state, &first, &owner, channel).await,
        Some(3)
    );
    assert_eq!(owner_spent(&state, &first, &owner).await, Some(3));
    for conn in [&first, &second, &fresh_key, &reconnect] {
        assert_eq!(counter(&state, conn, LimitType::Messages).await, None);
    }
    assert_metric(&recorder, "websocket", "quota", "agent_channel_events", 3);
}

/// Each channel has its own owner budget, and all of them draw on one
/// overall owner budget. Another owner's budgets are separate.
#[tokio::test]
#[ignore = "requires explicit isolated BUZZ_TEST_REDIS_URL"]
async fn channel_budgets_are_independent_under_one_owner_budget() {
    let state = state_with_limits(Limits {
        agent_channel: 3,
        agent_owner: 7,
        ..Limits::default()
    })
    .await;
    let recorder = DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let owner = Keys::generate().public_key();
    let (conn, mut rx) = agent_connection(&state, owner);
    let [a, b, c, d] = [(); 4].map(|()| Some(Uuid::new_v4()));

    for _ in 0..3 {
        assert!(admit(&channel_event(9, a), &conn, &mut rx, &state).await);
    }
    assert!(!admit(&channel_event(9, a), &conn, &mut rx, &state).await);
    for _ in 0..3 {
        assert!(admit(&channel_event(9, b), &conn, &mut rx, &state).await);
    }
    // The seventh event fills the owner budget with room left in channel c.
    assert!(admit(&channel_event(9, c), &conn, &mut rx, &state).await);
    assert!(!admit(&channel_event(9, c), &conn, &mut rx, &state).await);
    assert!(!admit(&channel_event(9, d), &conn, &mut rx, &state).await);
    // With both budgets full, the owner budget is the one reported.
    assert!(!admit(&channel_event(9, a), &conn, &mut rx, &state).await);

    assert_eq!(channel_spent(&state, &conn, &owner, a).await, Some(3));
    assert_eq!(channel_spent(&state, &conn, &owner, b).await, Some(3));
    assert_eq!(channel_spent(&state, &conn, &owner, c).await, Some(1));
    assert_eq!(channel_spent(&state, &conn, &owner, d).await, None);
    assert_eq!(owner_spent(&state, &conn, &owner).await, Some(7));
    assert_metrics(
        &recorder,
        &[
            ("websocket", "quota", "agent_channel_events", 1),
            ("websocket", "quota", "agent_owner_events", 3),
        ],
    );

    // Another owner's agent in the same community and channel is unaffected.
    let other_owner = Keys::generate().public_key();
    let (other, mut other_rx) = agent_socket(
        &state,
        conn.tenant.community(),
        Keys::generate().public_key(),
        other_owner,
    );
    assert!(admit(&channel_event(9, a), &other, &mut other_rx, &state).await);
    assert_eq!(owner_spent(&state, &conn, &owner).await, Some(7));
}

/// No channel, a channel tag on a kind that is never channel-scoped, and
/// invented channel ids all leave the owner's overall budget in force.
#[tokio::test]
#[ignore = "requires explicit isolated BUZZ_TEST_REDIS_URL"]
async fn absent_and_forged_channels_still_spend_the_owner_budget() {
    let state = state_with_limits(Limits {
        agent_channel: 2,
        agent_owner: 5,
        ..Limits::default()
    })
    .await;
    let owner = Keys::generate().public_key();
    let (conn, mut rx) = agent_connection(&state, owner);

    // Events naming no channel share one budget: a message without `h` and
    // a deletion without `h`.
    assert!(admit(&event(9), &conn, &mut rx, &state).await);
    assert!(admit(&event(5), &conn, &mut rx, &state).await);
    assert!(!admit(&event(9), &conn, &mut rx, &state).await);
    // A profile is never channel-scoped, so its `h` tag does not move it to
    // that channel's budget.
    let tagged_profile = channel_event(0, Some(Uuid::new_v4()));
    assert!(!admit(&tagged_profile, &conn, &mut rx, &state).await);
    assert_eq!(channel_spent(&state, &conn, &owner, None).await, Some(2));

    // Each invented channel has room, but the owner budget does not.
    for _ in 0..3 {
        assert!(
            admit(
                &channel_event(9, Some(Uuid::new_v4())),
                &conn,
                &mut rx,
                &state
            )
            .await
        );
    }
    assert!(
        !admit(
            &channel_event(9, Some(Uuid::new_v4())),
            &conn,
            &mut rx,
            &state
        )
        .await
    );
    assert_eq!(owner_spent(&state, &conn, &owner).await, Some(5));
}

#[tokio::test]
#[ignore = "requires explicit isolated BUZZ_TEST_REDIS_URL"]
async fn agent_budgets_are_isolated_by_community() {
    let state = state_with_limits(Limits {
        agent_channel: 1,
        agent_owner: 1,
        ..Limits::default()
    })
    .await;
    let owner = Keys::generate().public_key();
    let agent = Keys::generate().public_key();
    let channel = Some(Uuid::new_v4());
    let (here, mut here_rx) =
        agent_socket(&state, CommunityId::from_uuid(Uuid::new_v4()), agent, owner);
    let (there, mut there_rx) =
        agent_socket(&state, CommunityId::from_uuid(Uuid::new_v4()), agent, owner);

    assert!(admit(&channel_event(9, channel), &here, &mut here_rx, &state).await);
    assert!(!admit(&channel_event(9, channel), &here, &mut here_rx, &state).await);
    assert!(admit(&channel_event(9, channel), &there, &mut there_rx, &state).await);
    assert_eq!(owner_spent(&state, &here, &owner).await, Some(1));
    assert_eq!(owner_spent(&state, &there, &owner).await, Some(1));
    // Both budgets are one-minute windows.
    for key in [
        buzz_auth::agent_channel_budget_key(&there.tenant, &owner, channel),
        buzz_auth::agent_owner_budget_key(&there.tenant, &owner),
    ] {
        assert!((1..=60).contains(&ttl(&state, &key).await), "{key}");
    }
}

/// Two relay instances with their own connection pools admit concurrently
/// against the same Redis. Neither budget is ever overspent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicit isolated BUZZ_TEST_REDIS_URL"]
async fn concurrent_admission_across_relay_instances_never_overspends() {
    let limits = || Limits {
        agent_channel: 10,
        agent_owner: 15,
        ws: 1_000,
        ..Limits::default()
    };
    let instances = [
        Arc::new(state_with_limits(limits()).await),
        Arc::new(state_with_limits(limits()).await),
    ];
    assert!(!Arc::ptr_eq(
        &instances[0].admission_rate_limiter,
        &instances[1].admission_rate_limiter
    ));
    let community = CommunityId::from_uuid(Uuid::new_v4());
    let owner = Keys::generate().public_key();
    // Two agent keys per instance, all of one owner. Receivers stay alive so
    // refusals do not close the sockets mid-test.
    let mut receivers = Vec::new();
    let sockets: Vec<_> = (0..4)
        .map(|i| {
            let state = Arc::clone(&instances[i % 2]);
            let (conn, rx) = agent_socket(&state, community, Keys::generate().public_key(), owner);
            receivers.push(rx);
            (state, conn)
        })
        .collect();

    // Spawns every attempt before awaiting any, so they race in Redis.
    let race = |channels: Vec<Option<Uuid>>| {
        let attempts: Vec<_> = channels
            .into_iter()
            .enumerate()
            .map(|(i, channel)| {
                let (state, conn) = sockets[i % sockets.len()].clone();
                tokio::spawn(async move {
                    enforce_ws_admission(&channel_event(9, channel), &conn, &state).await
                })
            })
            .collect();
        async move {
            let mut admitted = 0;
            for attempt in attempts {
                admitted += usize::from(attempt.await.unwrap());
            }
            admitted
        }
    };

    let a = Some(Uuid::new_v4());
    assert_eq!(race(vec![a; 40]).await, 10, "channel budget");
    let (b, c) = (Some(Uuid::new_v4()), Some(Uuid::new_v4()));
    assert_eq!(race([b, c].repeat(20)).await, 5, "owner budget");

    let conn = &sockets[0].1;
    assert_eq!(
        channel_spent(&instances[0], conn, &owner, a).await,
        Some(10)
    );
    assert_eq!(owner_spent(&instances[1], conn, &owner).await, Some(15));
}

#[tokio::test]
#[ignore = "requires explicit isolated BUZZ_TEST_REDIS_URL"]
async fn ws_flood_budget_is_shared_by_ephemeral_stored_req_and_count() {
    let state = state_with_limits(Limits {
        messages: 10,
        ws: 1, // Five operations per window.
        ..Limits::default()
    })
    .await;
    let recorder = DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let (conn, mut rx) = connection();
    let req = ClientMessage::parse(r#"["REQ","history",{"kinds":[9]}]"#).unwrap();
    let count = ClientMessage::parse(r#"["COUNT","count",{"kinds":[9]}]"#).unwrap();
    for msg in [
        event(20001),
        event(20002),
        event(9),
        req.clone(),
        count.clone(),
    ] {
        assert!(enforce_ws_admission(&msg, &conn, &state).await);
    }
    assert_eq!(counter(&state, &conn, LimitType::Messages).await, Some(1));
    for msg in [event(24200), event(9), req, count] {
        assert!(!enforce_ws_admission(&msg, &conn, &state).await);
        assert_quota_frame(&read_frame(&mut rx), &msg, 5);
    }
    assert_eq!(counter(&state, &conn, LimitType::Messages).await, Some(1));
    assert_metric(&recorder, "websocket", "quota", "ws_operations", 4);
}

/// The per-key flood budget still applies to an agent ahead of its owner's
/// budgets, and an event it refuses spends neither owner budget.
#[tokio::test]
#[ignore = "requires explicit isolated BUZZ_TEST_REDIS_URL"]
async fn agent_flood_refusal_spends_no_owner_budget() {
    let state = state_with_limits(Limits {
        ws: 1, // Five operations per window.
        ..Limits::default()
    })
    .await;
    let owner = Keys::generate().public_key();
    let (conn, mut rx) = agent_connection(&state, owner);
    for _ in 0..5 {
        assert!(admit(&event(9), &conn, &mut rx, &state).await);
    }
    assert!(!admit(&event(9), &conn, &mut rx, &state).await);
    assert_eq!(owner_spent(&state, &conn, &owner).await, Some(5));
    assert_eq!(counter(&state, &conn, LimitType::WsEvents).await, Some(6));
}

#[tokio::test]
#[ignore = "requires explicit isolated BUZZ_TEST_REDIS_URL"]
async fn http_quota_preserves_wire_error_and_reports_api_bucket() {
    let state = state_with_limits(Limits {
        api_calls: 2,
        ..Limits::default()
    })
    .await;
    let recorder = DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let (conn, _) = connection();
    for _ in 0..2 {
        crate::api::bridge::enforce_http_admission(&state, &conn.tenant, &pubkey(&conn))
            .await
            .unwrap();
    }
    let (status, body) =
        crate::api::bridge::enforce_http_admission(&state, &conn.tenant, &pubkey(&conn))
            .await
            .unwrap_err();
    assert_eq!(status, axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert!(retry_seconds(body.0["error"].as_str().unwrap()) <= 60);
    assert_eq!(counter(&state, &conn, LimitType::Messages).await, None);
    assert_eq!(counter(&state, &conn, LimitType::WsEvents).await, None);
    assert_metric(&recorder, "http", "quota", "api_calls", 1);
}

/// `POST /events` with a router, as a client sends it: dev-mode `x-pubkey`
/// authentication on an open relay.
async fn post_event(
    state: &Arc<AppState>,
    host: &str,
    publisher: &nostr::PublicKey,
    msg: &ClientMessage,
) -> (axum::http::StatusCode, serde_json::Value) {
    send_event(state, host, msg, |request| {
        request.header("x-pubkey", publisher.to_hex())
    })
    .await
}

/// `POST /events` signed with NIP-98 by `agent`, presenting `owner`'s NIP-OA
/// attestation for it in `x-auth-tag`.
async fn post_attested_event(
    state: &Arc<AppState>,
    tenant: &buzz_core::TenantContext,
    agent: &Keys,
    owner: &Keys,
    msg: &ClientMessage,
) -> (axum::http::StatusCode, serde_json::Value) {
    let ClientMessage::Event(event) = msg else {
        unreachable!()
    };
    let url = crate::api::bridge::nip98_expected_url(&state.config.relay_url, tenant, "/events");
    let authorization =
        crate::nip98::nip98_header(agent, &url, &serde_json::to_vec(event).unwrap()).unwrap();
    let auth_tag = buzz_sdk::nip_oa::compute_auth_tag(owner, &agent.public_key(), "").unwrap();
    send_event(state, tenant.host(), msg, |request| {
        request
            .header(axum::http::header::AUTHORIZATION, authorization)
            .header("x-auth-tag", auth_tag)
    })
    .await
}

async fn send_event(
    state: &Arc<AppState>,
    host: &str,
    msg: &ClientMessage,
    authenticate: impl FnOnce(axum::http::request::Builder) -> axum::http::request::Builder,
) -> (axum::http::StatusCode, serde_json::Value) {
    use axum::http::{header, Request};
    use tower::ServiceExt;
    let ClientMessage::Event(event) = msg else {
        unreachable!()
    };
    let request = Request::builder()
        .method("POST")
        .uri("/events")
        .header(header::HOST, host);
    let response = crate::router::build_router(Arc::clone(state))
        .oneshot(
            authenticate(request)
                .body(axum::body::Body::from(serde_json::to_vec(event).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap_or_default())
}

/// HTTP and WebSocket events of one owner's agents spend the same budgets,
/// and an agent key is charged to its stored owner even when it publishes
/// without its auth tag. Keys with no owner keep the human HTTP behavior.
#[tokio::test]
#[ignore = "requires Postgres and explicit isolated BUZZ_TEST_REDIS_URL"]
async fn http_agent_events_share_owner_budgets_with_websocket() {
    let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("local Postgres and Redis");
    let state = Arc::new(with_limits(
        (*state).clone(),
        Limits {
            agent_channel: 2,
            agent_owner: 3,
            ..Limits::default()
        },
    ));
    let recorder = DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let host = format!("agent-budget-{}.local", Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("ensure community")
        .id;
    let tenant = buzz_core::TenantContext::resolved(community, &host);
    let owner = Keys::generate().public_key();
    let http_agent = Keys::generate().public_key();
    let (ws, mut ws_rx) = agent_socket(&state, community, Keys::generate().public_key(), owner);
    let channel = Some(Uuid::new_v4());

    // Before it has an owner, the key is not charged to any agent budget.
    let (status, _) = post_event(&state, &host, &http_agent, &channel_event(9, channel)).await;
    assert_ne!(status, axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(owner_spent(&state, &ws, &owner).await, None);
    // Another relay instance records the owner. This instance learns of it
    // on the next event, with no cache invalidation.
    link_in_database(&state, community, &http_agent, &owner).await;

    assert!(admit(&channel_event(9, channel), &ws, &mut ws_rx, &state).await);
    // Admitted by the budget; ingest then refuses the unknown channel.
    let (status, _) = post_event(&state, &host, &http_agent, &channel_event(9, channel)).await;
    assert_ne!(status, axum::http::StatusCode::TOO_MANY_REQUESTS);
    let (status, body) = post_event(&state, &host, &http_agent, &channel_event(9, channel)).await;
    assert_eq!(status, axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert!((1..=60).contains(&retry_seconds(body["error"].as_str().unwrap())));
    assert!(!admit(&channel_event(9, channel), &ws, &mut ws_rx, &state).await);

    // An ephemeral event over HTTP is not charged.
    let (status, _) = post_event(&state, &host, &http_agent, &channel_event(20002, channel)).await;
    assert_ne!(status, axum::http::StatusCode::TOO_MANY_REQUESTS);
    // The third event fills the owner budget from another channel.
    let (status, _) = post_event(&state, &host, &http_agent, &event(9)).await;
    assert_ne!(status, axum::http::StatusCode::TOO_MANY_REQUESTS);
    let (status, _) = post_event(
        &state,
        &host,
        &http_agent,
        &channel_event(9, Some(Uuid::new_v4())),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(channel_spent(&state, &ws, &owner, channel).await, Some(2));
    assert_eq!(owner_spent(&state, &ws, &owner).await, Some(3));
    assert_metrics(
        &recorder,
        &[
            ("http", "quota", "agent_channel_events", 1),
            ("http", "quota", "agent_owner_events", 1),
        ],
    );

    // A key with no owner is not charged to any agent budget over HTTP.
    let human = Keys::generate().public_key();
    for _ in 0..2 {
        let (status, _) = post_event(&state, &host, &human, &channel_event(9, channel)).await;
        assert_ne!(status, axum::http::StatusCode::TOO_MANY_REQUESTS);
    }
    let human_key = |bucket| buzz_auth::rate_limit::rate_limit_key(&tenant, &human, &bucket);
    assert_eq!(get(&state, human_key(LimitType::Messages)).await, None);
    assert_eq!(get(&state, human_key(LimitType::ApiCalls)).await, Some(2));
}

/// Records `owner` for `agent` directly in the database, as another relay
/// instance does, without touching this instance's caches.
async fn link_in_database(
    state: &AppState,
    community: CommunityId,
    agent: &nostr::PublicKey,
    owner: &nostr::PublicKey,
) {
    for key in [agent, owner] {
        state
            .db
            .ensure_user_for_authorization(community, key.as_bytes())
            .await
            .unwrap();
    }
    assert!(state
        .db
        .set_agent_owner_for_authorization(community, agent.as_bytes(), owner.as_bytes())
        .await
        .unwrap());
}

/// An HTTP agent proving its owner with a NIP-OA tag is charged to that owner
/// once the link is recorded. A key that already has a stored owner stays
/// charged to it when it presents another owner's tag.
#[tokio::test]
#[ignore = "requires Postgres and explicit isolated BUZZ_TEST_REDIS_URL"]
async fn http_nip_oa_agents_are_charged_to_their_stored_owner() {
    let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("local Postgres and Redis");
    let state = Arc::new(with_limits(
        (*state).clone(),
        Limits {
            agent_channel: 100,
            agent_owner: 100,
            ..Limits::default()
        },
    ));
    let host = format!("agent-budget-{}.local", Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("ensure community")
        .id;
    let tenant = buzz_core::TenantContext::resolved(community, &host);
    let (owner, other_owner) = (Keys::generate(), Keys::generate());
    let (ws, _ws_rx) = agent_socket(
        &state,
        community,
        Keys::generate().public_key(),
        owner.public_key(),
    );

    // A new agent: its first attested event records the owner and is charged.
    let agent = Keys::generate();
    let (status, _) =
        post_attested_event(&state, &tenant, &agent, &owner, &channel_event(9, None)).await;
    assert_ne!(status, axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert!(state
        .db
        .is_agent_owner(
            community,
            agent.public_key().as_bytes(),
            owner.public_key().as_bytes()
        )
        .await
        .unwrap());
    assert_eq!(owner_spent(&state, &ws, &owner.public_key()).await, Some(1));

    // The same agent presenting another owner's tag still spends the first.
    let (status, _) = post_attested_event(
        &state,
        &tenant,
        &agent,
        &other_owner,
        &channel_event(9, None),
    )
    .await;
    assert_ne!(status, axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(owner_spent(&state, &ws, &owner.public_key()).await, Some(2));
    assert_eq!(
        owner_spent(&state, &ws, &other_owner.public_key()).await,
        None
    );
}

#[tokio::test]
async fn ephemeral_and_http_admission_still_fail_closed_when_redis_is_unavailable() {
    let state = crate::state::tests::test_state().await;
    let recorder = DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let (conn, mut rx) = connection();
    let msg = event(20002);
    assert!(!enforce_ws_admission(&msg, &conn, &state).await);
    let ClientMessage::Event(event) = msg else {
        unreachable!()
    };
    assert_eq!(
        read_frame(&mut rx),
        serde_json::json!([
            "OK",
            event.id.to_hex(),
            false,
            "rate-limited: shared admission unavailable"
        ])
    );
    assert_metric(&recorder, "websocket", "unavailable", "ws_operations", 1);
    let (status, body) =
        crate::api::bridge::enforce_http_admission(&state, &conn.tenant, &pubkey(&conn))
            .await
            .unwrap_err();
    assert_eq!(status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body.0["error"],
        "rate-limited: shared admission unavailable"
    );
    assert_metric(&recorder, "http", "unavailable", "api_calls", 1);
}

/// Each NIP-OA link outcome on `POST /events` maps to the owner to charge:
/// the proven owner once linked, the token or stored owner on a conflict,
/// and a refusal when the link is unavailable.
#[test]
fn http_owner_link_outcomes_map_to_the_owner_to_charge() {
    use crate::api::bridge::linked_agent_owner;
    use crate::api::relay_members::OwnerLink;
    let (proven, token) = (Keys::generate().public_key(), Keys::generate().public_key());
    for (token_owner, link, expected) in [
        (None, None, Ok(None)),
        (Some(token), None, Ok(Some(token))),
        (None, Some(OwnerLink::Linked), Ok(Some(proven))),
        (None, Some(OwnerLink::Conflict), Ok(None)),
        (Some(token), Some(OwnerLink::Conflict), Ok(Some(token))),
        (None, Some(OwnerLink::Unavailable), Err(())),
        (Some(token), Some(OwnerLink::Unavailable), Err(())),
    ] {
        let nip_oa_link = link.map(|link| (proven, link));
        assert_eq!(
            linked_agent_owner(token_owner, nip_oa_link),
            expected,
            "{token_owner:?} {link:?}"
        );
    }
}

/// An HTTP agent event is refused, never let through uncounted, when the
/// shared counter, the proven owner's link, or the stored owner is
/// unavailable.
#[tokio::test]
async fn http_agent_event_admission_fails_closed_when_its_owner_or_counter_is_unavailable() {
    let state = crate::state::tests::test_state().await;
    let recorder = DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let owner = Keys::generate().public_key();
    let (conn, _) = agent_connection(&state, owner);
    let ClientMessage::Event(event) = channel_event(9, Some(Uuid::new_v4())) else {
        unreachable!()
    };
    let refuse = |state: Arc<AppState>, linked_owner| {
        let (tenant, pubkey, event) = (conn.tenant.clone(), pubkey(&conn), event.clone());
        async move {
            let (status, body) = crate::api::bridge::enforce_http_agent_event_admission(
                &state,
                &tenant,
                &pubkey,
                linked_owner,
                &event,
            )
            .await
            .unwrap_err();
            assert_eq!(status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(
                body.0["error"],
                "rate-limited: shared admission unavailable"
            );
        }
    };
    // The owner is known but its counter is not reachable.
    refuse(Arc::clone(&state), Ok(Some(owner))).await;
    // The request proved an owner whose link could not be read or written.
    let unavailable = crate::api::bridge::linked_agent_owner(
        None,
        Some((owner, crate::api::relay_members::OwnerLink::Unavailable)),
    );
    refuse(Arc::clone(&state), unavailable).await;
    // No owner was proven and the stored owner cannot be read.
    let no_database = crate::state::tests::test_state_with_unreachable_database_url(
        "postgres://127.0.0.1:1/none",
    )
    .await;
    refuse(no_database, Ok(None)).await;
    assert_metric(&recorder, "http", "unavailable", "agent_events", 3);
}

/// A WebSocket agent event is refused when the owner budgets cannot be
/// checked, even though the per-key flood check admitted it.
#[tokio::test]
#[ignore = "requires explicit isolated BUZZ_TEST_REDIS_URL"]
async fn ws_agent_event_admission_fails_closed_when_the_owner_check_fails() {
    let state = state_with_limits(Limits::default()).await;
    let recorder = DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let owner = Keys::generate().public_key();
    let (conn, mut rx) = agent_connection(&state, owner);
    // A value of the wrong type makes the owner budget script fail.
    let mut redis = state.redis_pool.get().await.unwrap();
    let key = buzz_auth::agent_owner_budget_key(&conn.tenant, &owner);
    let () = redis::cmd("HSET")
        .arg(&key)
        .arg("field")
        .arg("value")
        .query_async(&mut *redis)
        .await
        .unwrap();
    let () = redis::cmd("EXPIRE")
        .arg(&key)
        .arg(60)
        .query_async(&mut *redis)
        .await
        .unwrap();

    assert!(!enforce_ws_admission(&event(9), &conn, &state).await);
    assert_eq!(
        read_frame(&mut rx)[3],
        "rate-limited: shared admission unavailable"
    );
    assert_eq!(counter(&state, &conn, LimitType::WsEvents).await, Some(1));
    assert_metric(&recorder, "websocket", "unavailable", "agent_events", 1);
}
