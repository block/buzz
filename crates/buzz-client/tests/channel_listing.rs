use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use buzz_client::{
    AuthContext, BuzzClient, ChannelScope, ChannelVisibility, ClientConfig, ClientError,
    CommunityEndpoint, ListChannelsRequest, RetryPolicy,
};
use nostr::{Event, JsonUtil, Keys};
use serde_json::{json, Value};

#[derive(Clone, Default)]
struct Capture {
    headers: Arc<Mutex<Vec<HeaderMap>>>,
    filters: Arc<Mutex<Vec<Value>>>,
}

async fn query(
    State(capture): State<Capture>,
    headers: HeaderMap,
    Json(filters): Json<Value>,
) -> Json<Value> {
    capture.headers.lock().expect("headers lock").push(headers);
    capture.filters.lock().expect("filters lock").push(filters);
    Json(json!([{
        "id": "a".repeat(64),
        "pubkey": "b".repeat(64),
        "created_at": 42,
        "kind": 39000,
        "content": "",
        "tags": [
            ["d", "11111111-1111-1111-1111-111111111111"],
            ["name", "general"],
            ["about", "General discussion"],
            ["public"]
        ],
        "sig": "c".repeat(128)
    }]))
}

#[tokio::test]
async fn listing_uses_async_nip98_and_exactly_one_ambient_header() {
    let capture = Capture::default();
    let app = Router::new()
        .route("/query", post(query))
        .with_state(capture.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test relay");
    });

    let owner = Keys::generate();
    let agent = Keys::generate();
    let auth_json = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "kind=9")
        .expect("auth tag");
    let client = BuzzClient::builder(
        CommunityEndpoint::parse(&format!("http://{address}")).expect("endpoint"),
        Arc::new(agent.clone()),
    )
    .auth_context(AuthContext::nip_oa(&auth_json).expect("auth context"))
    .build()
    .await
    .expect("client");

    let channels = client
        .list_channels(ListChannelsRequest {
            scope: ChannelScope::Visible,
            visibility: Some(ChannelVisibility::Open),
            limit: 10,
        })
        .await
        .expect("channel list");
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].name, "general");
    assert_eq!(
        channels[0].description.as_deref(),
        Some("General discussion")
    );

    let headers = capture.headers.lock().expect("headers lock");
    assert_eq!(headers.len(), 1);
    assert_eq!(
        headers[0]
            .get("x-auth-tag")
            .and_then(|value| value.to_str().ok()),
        Some(auth_json.as_str())
    );
    let authorization = headers[0]
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Nostr "))
        .expect("NIP-98 authorization");
    let event_json = BASE64_STANDARD
        .decode(authorization)
        .expect("base64 authorization");
    let event = Event::from_json(event_json).expect("NIP-98 event");
    event.verify().expect("valid NIP-98 signature");
    assert_eq!(event.pubkey, agent.public_key());
    assert_eq!(event.kind.as_u16(), 27235);
    assert_eq!(
        event
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("auth"))
            .count(),
        0
    );
}

#[tokio::test]
async fn equal_timestamp_pages_advance_with_composite_cursor() {
    let filters = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured_filters = filters.clone();
    let first_page: Vec<Value> = (0_u64..500)
        .map(|index| {
            json!({
                "id": format!("{index:064x}"),
                "created_at": 1000,
                "kind": 39000,
                "tags": [
                    ["d", format!("00000000-0000-0000-0000-{index:012}")],
                    ["name", format!("channel-{index}")],
                    ["public"]
                ]
            })
        })
        .collect();
    let second_page = vec![json!({
        "id": format!("{:064x}", 500_u64),
        "created_at": 1000,
        "kind": 39000,
        "tags": [
            ["d", "00000000-0000-0000-0000-000000000500"],
            ["name", "channel-500"],
            ["public"]
        ]
    })];
    let app = Router::new().route(
        "/query",
        post(move |Json(filter): Json<Value>| {
            let captured_filters = captured_filters.clone();
            let first_page = first_page.clone();
            let second_page = second_page.clone();
            async move {
                let paginated = filter[0].get("before_id").is_some();
                captured_filters.lock().expect("filters lock").push(filter);
                Json(if paginated { second_page } else { first_page })
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test relay");
    });
    let client = BuzzClient::builder(
        CommunityEndpoint::parse(&format!("http://{address}")).expect("endpoint"),
        Arc::new(Keys::generate()),
    )
    .build()
    .await
    .expect("client");

    let channels = client
        .list_channels(ListChannelsRequest {
            limit: 501,
            ..ListChannelsRequest::default()
        })
        .await
        .expect("channel list");
    assert_eq!(channels.len(), 501);
    let filters = filters.lock().expect("filters lock");
    assert_eq!(filters.len(), 2);
    assert_eq!(filters[1][0]["until"], 1000);
    assert_eq!(filters[1][0]["before_id"], format!("{:064x}", 499_u64));
}

#[tokio::test]
async fn member_scope_deduplicates_membership_ids_before_metadata_query() {
    let keys = Keys::generate();
    let public_key = keys.public_key().to_hex();
    let filters = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured_filters = filters.clone();
    let app = Router::new().route(
        "/query",
        post(move |Json(filter): Json<Value>| {
            let captured_filters = captured_filters.clone();
            let public_key = public_key.clone();
            async move {
                let membership = filter[0]["kinds"] == json!([39002]);
                captured_filters
                    .lock()
                    .expect("filters lock")
                    .push(filter);
                if membership {
                    Json(json!([
                        {
                            "id": "a".repeat(64),
                            "created_at": 2,
                            "tags": [["d", "11111111-1111-1111-1111-111111111111"], ["p", public_key]]
                        },
                        {
                            "id": "b".repeat(64),
                            "created_at": 1,
                            "tags": [["d", "11111111-1111-1111-1111-111111111111"], ["p", public_key]]
                        }
                    ]))
                } else {
                    Json(json!([{
                        "id": "c".repeat(64),
                        "created_at": 3,
                        "tags": [
                            ["d", "11111111-1111-1111-1111-111111111111"],
                            ["name", "member-channel"],
                            ["private"]
                        ]
                    }]))
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test relay");
    });
    let client = BuzzClient::builder(
        CommunityEndpoint::parse(&format!("http://{address}")).expect("endpoint"),
        Arc::new(keys),
    )
    .build()
    .await
    .expect("client");

    let channels = client
        .list_channels(ListChannelsRequest {
            scope: ChannelScope::Member,
            visibility: Some(ChannelVisibility::Private),
            limit: 10,
        })
        .await
        .expect("member channels");
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].name, "member-channel");
    let filters = filters.lock().expect("filters lock");
    assert_eq!(filters.len(), 2);
    assert_eq!(
        filters[1][0]["#d"],
        json!(["11111111-1111-1111-1111-111111111111"])
    );
}

#[tokio::test]
async fn relay_rejection_preserves_status_reason_and_retry_classification() {
    let app = Router::new().route(
        "/query",
        post(|| async {
            (
                axum::http::StatusCode::FORBIDDEN,
                Json(json!({"error": "membership required"})),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test relay");
    });
    let client = BuzzClient::builder(
        CommunityEndpoint::parse(&format!("http://{address}")).expect("endpoint"),
        Arc::new(Keys::generate()),
    )
    .build()
    .await
    .expect("client");

    let error = client
        .list_channels(ListChannelsRequest::default())
        .await
        .expect_err("forbidden query must fail");
    assert!(matches!(
        error,
        ClientError::Relay {
            status: 403,
            reason,
            retry_safe: false,
        } if reason == "membership required"
    ));
}

#[tokio::test]
async fn pre_relay_channel_network_failure_is_typed() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve address");
    let address = listener.local_addr().expect("address");
    drop(listener);
    let retry = RetryPolicy::new(1, Duration::from_millis(1), Duration::from_millis(1))
        .expect("retry policy");
    let config = ClientConfig::new(Duration::from_secs(1), Duration::from_secs(1), retry)
        .expect("client config");
    let client = BuzzClient::builder(
        CommunityEndpoint::parse(&format!("http://{address}")).expect("endpoint"),
        Arc::new(Keys::generate()),
    )
    .config(config)
    .build()
    .await
    .expect("client");

    let error = client
        .list_channels(ListChannelsRequest::default())
        .await
        .expect_err("unreachable relay must fail");
    assert!(matches!(error, ClientError::Network(_)));
}
