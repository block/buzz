//! Real Redis counters through the production admission entrypoints. Tests use
//! disposable identities and expiring keys, never FLUSHDB or a shared identity.

use super::*;
use axum::extract::ws::Message as WsMessage;
use buzz_auth::{AuthService, RateLimitConfig};
use buzz_pubsub::rate_limiter::RedisRateLimiter;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use nostr::{EventBuilder, Keys, Kind};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::connection::tests::{authenticated_state, read_frame, test_conn_with_auth};

async fn state_with_limits(messages: u64, agent_messages: u64, ws: u64) -> AppState {
    let url = std::env::var("BUZZ_TEST_REDIS_URL").expect("explicit isolated BUZZ_TEST_REDIS_URL");
    let mut state = (*crate::state::tests::test_state().await).clone();
    let mut auth = state.auth.config().clone();
    auth.rate_limits = RateLimitConfig {
        human_messages_per_min: messages,
        agent_standard_messages_per_min: agent_messages,
        human_ws_events_per_sec: ws,
        human_api_calls_per_min: 2,
        ..RateLimitConfig::default()
    };
    state.auth = Arc::new(AuthService::new(auth));
    state.redis_pool = deadpool_redis::Config::from_url(url)
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .unwrap();
    state.admission_rate_limiter = Arc::new(RedisRateLimiter::new(state.redis_pool.clone()));
    state
}

fn connection(agent: bool) -> (Arc<ConnectionState>, mpsc::Receiver<WsMessage>) {
    let AuthState::Authenticated(mut auth) = authenticated_state() else {
        unreachable!()
    };
    if agent {
        auth.agent_owner_pubkey = Some(Keys::generate().public_key());
    }
    test_conn_with_auth(AuthState::Authenticated(auth))
}

fn event(kind: u16) -> ClientMessage {
    let event = EventBuilder::new(Kind::from(kind), "")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    ClientMessage::parse(&serde_json::json!(["EVENT", event]).to_string()).unwrap()
}

fn pubkey(conn: &ConnectionState) -> nostr::PublicKey {
    let AuthState::Authenticated(auth) = conn.auth_state_snapshot() else {
        unreachable!()
    };
    auth.pubkey
}

async fn counter(state: &AppState, conn: &ConnectionState, bucket: LimitType) -> Option<u64> {
    let key = buzz_auth::rate_limit::rate_limit_key(&conn.tenant, &pubkey(conn), &bucket);
    let mut redis = state.redis_pool.get().await.unwrap();
    redis::cmd("GET")
        .arg(key)
        .query_async(&mut *redis)
        .await
        .unwrap()
}

fn assert_metric(
    recorder: &DebuggingRecorder,
    transport: &str,
    reason: &str,
    bucket: &str,
    count: u64,
) {
    let snapshot = recorder.snapshotter().snapshot().into_vec();
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
    assert_eq!(actual, Some(count));
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
    let seconds: u64 = reason
        .strip_prefix("rate-limited: quota exceeded; retry in ")
        .unwrap()
        .strip_suffix('s')
        .unwrap()
        .parse()
        .unwrap();
    assert!(seconds <= max_retry);
}

#[tokio::test]
#[ignore = "requires explicit isolated BUZZ_TEST_REDIS_URL"]
async fn ephemeral_events_leave_human_and_agent_message_budgets_untouched() {
    let state = state_with_limits(2, 3, 100).await;
    let recorder = DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    // Both range boundaries, typing, presence, and observer frames. Adjacent
    // non-ephemeral kinds below must still consume the message allowance.
    let ephemeral_kinds = [20000, 20001, 20002, 24200, 29999];
    for (agent, limit) in [(false, 2), (true, 3)] {
        for stored_kind in [0, 1, 9, 19999, 30000, 40002] {
            let (conn, mut rx) = connection(agent);
            for kind in ephemeral_kinds {
                assert!(enforce_ws_admission(&event(kind), &conn, &state).await);
            }
            assert_eq!(counter(&state, &conn, LimitType::Messages).await, None);
            assert_eq!(counter(&state, &conn, LimitType::WsEvents).await, Some(5));
            for expected in 1..=limit {
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
            assert_eq!(
                counter(&state, &conn, LimitType::Messages).await,
                Some(limit + 1)
            );
            assert!(rx.try_recv().is_err());
        }
    }
    assert_metric(&recorder, "websocket", "quota", "messages", 12);
}

#[tokio::test]
#[ignore = "requires explicit isolated BUZZ_TEST_REDIS_URL"]
async fn ws_flood_budget_is_shared_by_ephemeral_stored_req_and_count() {
    let state = state_with_limits(10, 20, 1).await; // Five operations per window.
    let recorder = DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let (conn, mut rx) = connection(false);
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

#[tokio::test]
#[ignore = "requires explicit isolated BUZZ_TEST_REDIS_URL"]
async fn http_quota_preserves_wire_error_and_reports_api_bucket() {
    let state = state_with_limits(0, 0, 0).await;
    let recorder = DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let (conn, _) = connection(false);
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
    let seconds: u64 = body.0["error"]
        .as_str()
        .unwrap()
        .strip_prefix("rate-limited: quota exceeded; retry in ")
        .unwrap()
        .strip_suffix('s')
        .unwrap()
        .parse()
        .unwrap();
    assert!(seconds <= 60);
    assert_eq!(counter(&state, &conn, LimitType::Messages).await, None);
    assert_eq!(counter(&state, &conn, LimitType::WsEvents).await, None);
    assert_metric(&recorder, "http", "quota", "api_calls", 1);
}

#[tokio::test]
async fn ephemeral_and_http_admission_still_fail_closed_when_redis_is_unavailable() {
    let state = crate::state::tests::test_state().await;
    let recorder = DebuggingRecorder::new();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let (conn, mut rx) = connection(false);
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
