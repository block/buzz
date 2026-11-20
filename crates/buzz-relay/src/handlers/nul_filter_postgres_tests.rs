//! Postgres-backed tests: every read path refuses a filter string holding a
//! NUL byte before it reaches Postgres, which cannot store U+0000 in `text`
//! or `jsonb` and fails the statement instead of matching nothing.

use std::time::Duration;

use axum::extract::ws::Message as WsMessage;
use axum::http::StatusCode;
use buzz_core::TenantContext;
use nostr::Keys;
use serde_json::{json, Value};
use uuid::Uuid;

use super::group_state_postgres_tests::test_conn;
use crate::protocol::ClientMessage;
use crate::state::AppState;

const NUL_CLOSED: &str = "invalid: filter contains a NUL byte";

struct Fixture {
    state: std::sync::Arc<AppState>,
    tenant: TenantContext,
    host: String,
    reader: Keys,
}

async fn fixture() -> Fixture {
    let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("local Postgres and Redis");
    let host = format!("nul-filter-{}.local", Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("ensure community")
        .id;
    Fixture {
        state,
        tenant: TenantContext::resolved(community, &host),
        host,
        reader: Keys::generate(),
    }
}

/// Send one `REQ`/`COUNT` frame through the production frame parser and its
/// handler, and return the first reply other than a NOTICE.
async fn ws(f: &Fixture, verb: &str, filter: &Value) -> Value {
    let (conn, mut rx) = test_conn(&f.tenant, &f.reader);
    let frame = json!([verb, "s", filter]).to_string();
    let state = f.state.clone();
    let task = match ClientMessage::parse(&frame).expect("frame parses") {
        ClientMessage::Req {
            sub_id,
            filters,
            before_ids,
        } => tokio::spawn(super::req::handle_req(
            sub_id,
            filters,
            before_ids,
            conn.clone(),
            state,
        )),
        ClientMessage::Count { sub_id, filters } => tokio::spawn(super::count::handle_count(
            sub_id,
            filters,
            conn.clone(),
            state,
        )),
        ClientMessage::ArtifactSync { sub_id, filter } => {
            let permit = state
                .handler_semaphore
                .clone()
                .try_acquire_owned()
                .expect("permit");
            tokio::spawn(super::artifact_sync::handle_sync(
                sub_id,
                filter,
                conn.clone(),
                state,
                permit,
            ))
        }
        _ => panic!("not a read frame: {frame}"),
    };
    let reply = loop {
        let message = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .unwrap_or_else(|_| panic!("no reply to {frame}"))
            .unwrap_or_else(|| panic!("connection dropped on {frame}"));
        let WsMessage::Text(text) = message else {
            continue;
        };
        let reply: Value = serde_json::from_str(&text).expect("relay frame");
        if reply[0] != "NOTICE" {
            break reply;
        }
    };
    task.abort();
    reply
}

/// POST `[filter]` to an HTTP read route; return the status and JSON body.
async fn http(f: &Fixture, path: &str, filter: &Value) -> (StatusCode, Value) {
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    let response = crate::router::build_router(f.state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(header::HOST, &f.host)
                .header("x-pubkey", f.reader.public_key().to_hex())
                .header("content-type", "application/json")
                .body(Body::from(json!([filter]).to_string()))
                .expect("request"),
        )
        .await
        .expect("router");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

/// A NUL byte in any free-form filter string is refused on WS REQ and COUNT
/// (CLOSED) and HTTP `/query` and `/count` (400), and the same filter without
/// it is served. Before the check, `#p`, `#e` and `#d` failed in Postgres
/// (CLOSED `error: database error`, HTTP 500).
/// Mutation: drop the `filters_contain_nul` check in any of the four paths, or
/// its `#buzz-channel` clause in the bridge → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn generic_read_paths_refuse_nul_bytes() {
    let f = fixture().await;
    let cases = |value: &str| {
        [
            ("#p", json!({"kinds": [9], "#p": [value]})),
            ("#e", json!({"kinds": [9], "#e": [value]})),
            ("#d", json!({"kinds": [30023], "#d": [value]})),
            ("#t", json!({"kinds": [9], "#t": [value]})),
            ("search", json!({"kinds": [9], "search": value})),
        ]
    };
    for (name, filter) in cases("a\u{0}b") {
        assert_eq!(
            ws(&f, "REQ", &filter).await,
            json!(["CLOSED", "s", NUL_CLOSED]),
            "WS REQ {name}"
        );
        assert_eq!(
            ws(&f, "COUNT", &filter).await,
            json!(["CLOSED", "s", NUL_CLOSED]),
            "WS COUNT {name}"
        );
        for path in ["/query", "/count"] {
            assert_eq!(
                http(&f, path, &filter).await,
                (StatusCode::BAD_REQUEST, json!({"error": NUL_CLOSED})),
                "HTTP {path} {name}"
            );
        }
    }
    // `#buzz-channel` is an HTTP-only extension that `nostr::Filter` drops;
    // the bridge binds it from the raw filter.
    for path in ["/query", "/count"] {
        let filter = json!({"kinds": [30621], "#buzz-channel": ["a\u{0}b"]});
        assert_eq!(
            http(&f, path, &filter).await,
            (StatusCode::BAD_REQUEST, json!({"error": NUL_CLOSED})),
            "HTTP {path} #buzz-channel"
        );
        let filter = json!({"kinds": [30621], "#buzz-channel": ["ab"]});
        assert_eq!(
            http(&f, path, &filter).await.0,
            StatusCode::OK,
            "HTTP {path} #buzz-channel"
        );
    }
    for (name, filter) in cases("ab") {
        assert_eq!(ws(&f, "REQ", &filter).await[0], "EOSE", "WS REQ {name}");
        assert_eq!(
            ws(&f, "COUNT", &filter).await[0],
            "COUNT",
            "WS COUNT {name}"
        );
        for path in ["/query", "/count"] {
            assert_eq!(
                http(&f, path, &filter).await.0,
                StatusCode::OK,
                "HTTP {path} {name}"
            );
        }
    }
}

/// NIP-AR filters are parsed apart from NIP-01 ones: a NUL byte in a
/// predicate value or name is refused on a WS sync (CLOSED) and an HTTP
/// artifact read (400). Before the check, the HTTP read failed in Postgres
/// (503). An incremental read wraps the same query and refuses it with its
/// normal 400 `incremental_invalid`.
/// Mutation: drop the NUL check in `parse_predicates` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn artifact_read_paths_refuse_nul_bytes() {
    let f = fixture().await;
    let channel = Uuid::new_v4().to_string();
    for (name, predicate) in [
        ("value", json!({"#type": ["a\u{0}b"]})),
        ("name", json!({"#ty\u{0}pe": ["a"]})),
    ] {
        let mut sync = json!({"artifact": "current", "#h": [channel]});
        let mut query = json!({"artifact": "current"});
        for filter in [&mut sync, &mut query] {
            filter
                .as_object_mut()
                .expect("object")
                .extend(predicate.as_object().expect("object").clone());
        }
        assert_eq!(
            ws(&f, "REQ", &sync).await,
            json!(["CLOSED", "s", NUL_CLOSED]),
            "WS sync {name}"
        );
        for path in ["/query", "/count"] {
            assert_eq!(
                http(&f, path, &query).await,
                (
                    StatusCode::BAD_REQUEST,
                    json!({"error": buzz_core::filter::NUL_BYTE_REJECTION})
                ),
                "HTTP {path} {name}"
            );
        }
        assert_eq!(
            http(
                &f,
                "/query",
                &json!({"artifact": "incremental", "query": query})
            )
            .await,
            (
                StatusCode::BAD_REQUEST,
                json!({"error": "incremental_invalid", "code": "incremental_invalid"})
            ),
            "HTTP incremental {name}"
        );
    }
    let (status, body) = http(
        &f,
        "/query",
        &json!({"artifact": "current", "#type": ["a"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "HTTP artifact control: {body}");
}
