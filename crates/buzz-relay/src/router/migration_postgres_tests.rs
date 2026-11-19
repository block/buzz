use std::{sync::Arc, time::Duration};

use axum::{
    body::{to_bytes, Body},
    http::{header, Method, Request, StatusCode},
    Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use buzz_core::kind::{KIND_NIP29_CREATE_GROUP, KIND_NIP29_PUT_USER, KIND_TYPING_INDICATOR};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Tag};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::{net::TcpListener, time::timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    api::media_access::MediaReadMode,
    config::{AdminAuth, AdminConfig, Config},
    handlers::group_state_postgres_tests::{community, create_channel},
    state::tests::build_test_state,
};

const ADMIN_HOST: &str = "admin.example.org";
const LIMIT_PATH: &str = "/api/admin/v1/migration-write-limit";
const TEST_LIMIT_KEY: &str = "buzz:ratelimit:migration_writes";

fn signed_request(
    host: &str,
    method: Method,
    path: &str,
    keys: Option<&Keys>,
    body: Vec<u8>,
) -> Request<Body> {
    let mut request = Request::builder()
        .method(method.clone())
        .uri(path)
        .header(header::HOST, host);
    if let Some(keys) = keys {
        let scheme = if host == ADMIN_HOST { "https" } else { "http" };
        let mut tags = vec![
            Tag::parse(["u", &format!("{scheme}://{host}{path}")]).unwrap(),
            Tag::parse(["method", method.as_str()]).unwrap(),
            Tag::parse(["client", &Uuid::new_v4().to_string()]).unwrap(),
        ];
        if method == Method::POST || method == Method::PUT {
            tags.push(Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap());
        }
        let proof = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap();
        request = request.header(
            header::AUTHORIZATION,
            format!("Nostr {}", STANDARD.encode(proof.as_json())),
        );
    }
    request.body(Body::from(body)).unwrap()
}

async fn request_json(
    app: &Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let bytes = to_bytes(body, 4096).await.unwrap();
    (
        parts.status,
        parts.headers,
        serde_json::from_slice(&bytes).unwrap(),
    )
}

async fn admin(
    app: &Router,
    method: Method,
    keys: Option<&Keys>,
    rate: Option<u32>,
) -> (StatusCode, Value) {
    let body = rate.map_or_else(Vec::new, |rate| {
        json!({"eventsPerMinute": rate}).to_string().into_bytes()
    });
    let (status, _, body) = request_json(
        app,
        signed_request(ADMIN_HOST, method, LIMIT_PATH, keys, body),
    )
    .await;
    (status, body)
}

async fn add_over_http(
    app: &Router,
    host: &str,
    keys: &Keys,
    event: &Event,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    request_json(
        app,
        signed_request(
            host,
            Method::POST,
            "/events",
            Some(keys),
            event.as_json().into_bytes(),
        ),
    )
    .await
}

fn add_members(
    keys: &Keys,
    channel: Uuid,
    recipients: &[PublicKey],
    source: Option<&str>,
) -> Event {
    let mut tags = vec![Tag::parse(["h", &channel.to_string()]).unwrap()];
    tags.extend(recipients.iter().copied().map(Tag::public_key));
    if let Some(source) = source {
        tags.push(Tag::parse(["migration", source]).unwrap());
    }
    EventBuilder::new(Kind::Custom(KIND_NIP29_PUT_USER as u16), "")
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

fn create_migration_channel(keys: &Keys, channel: Uuid, about: &str) -> Event {
    EventBuilder::new(Kind::Custom(KIND_NIP29_CREATE_GROUP as u16), "")
        .tags([
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse(["name", &format!("example-import-{channel}")]).unwrap(),
            Tag::parse(["migration", "slack"]).unwrap(),
            Tag::parse(["about", about]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap()
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn receive(socket: &mut Socket, frame_type: &str) -> Value {
    timeout(Duration::from_secs(8), async {
        loop {
            let message = socket.next().await.expect("WS frame").expect("WS receive");
            if let Message::Text(body) = message {
                let frame: Value = serde_json::from_str(&body).unwrap();
                if frame[0] == frame_type {
                    return frame;
                }
            }
        }
    })
    .await
    .expect("WS response deadline")
}

async fn send_event(socket: &mut Socket, frame_type: &str, event: &Event) -> Value {
    socket
        .send(Message::Text(json!([frame_type, event]).to_string().into()))
        .await
        .unwrap();
    let reply = receive(socket, "OK").await;
    assert_eq!(reply[1], event.id.to_hex());
    reply
}

async fn start_ws(app: Router, host: &str, keys: &Keys) -> (Socket, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let mut request = format!("ws://{addr}/").into_client_request().unwrap();
    request
        .headers_mut()
        .insert(header::HOST, host.parse().unwrap());
    let (mut socket, _) = connect_async(request).await.unwrap();
    let challenge = receive(&mut socket, "AUTH").await;
    let auth = EventBuilder::auth(
        challenge[1].as_str().unwrap(),
        format!("ws://{host}").parse().unwrap(),
    )
    .sign_with_keys(keys)
    .unwrap();
    assert_eq!(send_event(&mut socket, "AUTH", &auth).await[2], true);
    (socket, server)
}

#[tokio::test]
#[ignore = "requires Postgres and Redis — runs in postgres-ci nextest lane"]
async fn migration_write_limit_controls_http_and_ws_across_relay_instances() {
    // Keep the deployment-wide setting isolated from other integration tests.
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    let mut redis_url = url::Url::parse(&redis_url).unwrap();
    assert!(matches!(
        redis_url.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]")
    ));
    redis_url.set_path("/15");

    let operator = Keys::generate();
    let moderator = Keys::generate();
    let owner_a = Keys::generate();
    let owner_b = Keys::generate();
    let channel_member = Keys::generate();
    let mut config = Config::for_test();
    config.require_auth_token = false;
    config.require_relay_membership = false;
    config.relay_url = "ws://localhost:3000".into();
    config.redis_url = redis_url.to_string();
    config.media_read_mode = MediaReadMode::Shadow;
    config.relay_operator_pubkeys = vec![operator.public_key().to_hex()];
    config.admin = Some(AdminConfig {
        host: ADMIN_HOST.into(),
        auth: AdminAuth::Nip98,
        web_dir: None,
    });
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let state_a = build_test_state(config.clone(), pool.clone()).await;
    let state_b = build_test_state(config, pool).await;
    // Reset this fixture's value after an interrupted prior run.
    let _: () = redis::cmd("DEL")
        .arg(TEST_LIMIT_KEY)
        .query_async(&mut state_a.redis_pool.clone())
        .await
        .unwrap();
    state_a
        .db
        .upsert_relay_operator(
            moderator.public_key().as_bytes(),
            "moderator",
            operator.public_key().as_bytes(),
            true,
        )
        .await
        .unwrap();
    let tenant_a = community(&state_a, "migration-a").await;
    let tenant_b = community(&state_b, "migration-b").await;
    let channel_a = Uuid::new_v4();
    let channel_b = create_channel(
        &state_b,
        &tenant_b,
        &owner_b,
        &[&["p", &channel_member.public_key().to_hex()]],
    )
    .await;
    let app_a = super::build_router(Arc::clone(&state_a));
    let app_b = super::build_router(Arc::clone(&state_b));
    let (mut socket, server) = start_ws(app_b.clone(), tenant_b.host(), &owner_b).await;

    assert_eq!(
        admin(&app_a, Method::GET, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        admin(&app_a, Method::PUT, None, Some(0)).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        admin(&app_a, Method::GET, Some(&moderator), None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        admin(&app_a, Method::PUT, Some(&moderator), Some(0))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let mut disabled = (*state_a).clone();
    Arc::make_mut(&mut disabled.config)
        .admin
        .as_mut()
        .unwrap()
        .auth = AdminAuth::Disabled;
    let disabled = super::build_router(Arc::new(disabled));
    assert_eq!(
        admin(&disabled, Method::GET, None, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        admin(&disabled, Method::PUT, None, Some(0)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        admin(&app_a, Method::PUT, Some(&operator), Some(60_001))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let tampered = signed_request(
        ADMIN_HOST,
        Method::PUT,
        LIMIT_PATH,
        Some(&operator),
        br#"{"eventsPerMinute":0}"#.to_vec(),
    )
    .map(|_| Body::from(r#"{"eventsPerMinute":1}"#));
    assert_eq!(
        request_json(&app_a, tampered).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        admin(&app_b, Method::GET, Some(&operator), None).await,
        (StatusCode::OK, json!({"eventsPerMinute": 12}))
    );
    assert_eq!(
        admin(&app_a, Method::PUT, Some(&operator), Some(1)).await.0,
        StatusCode::OK
    );

    // A member passes channel membership but cannot grant ownership. Neither
    // ingress may charge this rejected write against the two available permits.
    let denied_add = EventBuilder::new(Kind::Custom(KIND_NIP29_PUT_USER as u16), "")
        .tags([
            Tag::parse(["h", &channel_b.to_string()]).unwrap(),
            Tag::parse(["p", &Keys::generate().public_key().to_hex(), "owner"]).unwrap(),
            Tag::parse(["migration", "example-import"]).unwrap(),
        ])
        .sign_with_keys(&channel_member)
        .unwrap();
    let (status, _, body) =
        add_over_http(&app_b, tenant_b.host(), &channel_member, &denied_add).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (mut member_socket, member_server) =
        start_ws(app_b.clone(), tenant_b.host(), &channel_member).await;
    let denied = send_event(&mut member_socket, "EVENT", &denied_add).await;
    assert_eq!(denied[2], false);
    assert_eq!(denied[3], body["error"]);
    assert!(denied[3].as_str().unwrap().contains("owners"));
    drop(member_socket);
    member_server.abort();

    // The HTTP typing bridge and WebSocket ephemeral path bypass ingest.
    // They must refuse a labeled write outside the supported migration kinds.
    let labeled_typing = EventBuilder::new(Kind::Custom(KIND_TYPING_INDICATOR as u16), "")
        .tags([
            Tag::parse(["h", &channel_b.to_string()]).unwrap(),
            Tag::parse(["migration", "example-import"]).unwrap(),
        ])
        .sign_with_keys(&owner_b)
        .unwrap();
    let (status, _, body) = add_over_http(&app_b, tenant_b.host(), &owner_b, &labeled_typing).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let denied = send_event(&mut socket, "EVENT", &labeled_typing).await;
    assert_eq!(denied[2], false);
    assert_eq!(denied[3], body["error"]);
    assert_eq!(
        denied[3],
        "invalid: migration tags require a channel create or membership add"
    );

    let reserved_create = create_migration_channel(&owner_a, Uuid::nil(), "");
    let (status, _, body) =
        add_over_http(&app_a, tenant_a.host(), &owner_a, &reserved_create).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body, json!({"error": "invalid: channel ID is reserved"}));

    // The importer sends the create and first batch back-to-back. A one-event
    // bucket would leave a created channel with no imported members.
    let create_event = create_migration_channel(&owner_a, channel_a, "");
    let (status, _, body) = add_over_http(&app_a, tenant_a.host(), &owner_a, &create_event).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["accepted"], true);
    let recipients = [Keys::generate().public_key(), Keys::generate().public_key()];
    let first = add_members(&owner_a, channel_a, &recipients, Some("slack"));
    let mut tampered = first.clone();
    tampered.content = "changed after signing".into();
    let (status, _, body) = add_over_http(&app_a, tenant_a.host(), &owner_a, &tampered).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, _, body) = add_over_http(&app_a, tenant_a.host(), &owner_a, &first).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["accepted"], true);
    for recipient in recipients {
        assert!(state_a
            .db
            .get_member_role(tenant_a.community(), channel_a, recipient.as_bytes())
            .await
            .unwrap()
            .is_some());
    }

    let ws_event = add_members(&owner_b, channel_b, &recipients, Some("example-import"));
    let refused = send_event(&mut socket, "EVENT", &ws_event).await;
    assert_eq!(refused[2], false);
    assert!(refused[3]
        .as_str()
        .unwrap()
        .starts_with("rate-limited: quota exceeded; retry in "));

    let http_event = add_members(
        &owner_a,
        channel_a,
        &[Keys::generate().public_key()],
        Some("slack"),
    );
    let (status, headers, body) =
        add_over_http(&app_a, tenant_a.host(), &owner_a, &http_event).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let retry = headers[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!((1..=60).contains(&retry));
    assert_eq!(
        body,
        json!({"error": format!("rate-limited: quota exceeded; retry in {retry}s")})
    );

    assert_eq!(
        admin(&app_a, Method::PUT, Some(&operator), Some(0)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        admin(&app_b, Method::GET, Some(&operator), None).await,
        (StatusCode::OK, json!({"eventsPerMinute": 0}))
    );
    let paused = send_event(&mut socket, "EVENT", &ws_event).await;
    assert_eq!(paused[2], false);
    assert_eq!(paused[3], "rate-limited: quota exceeded; retry in 5s");
    let (status, headers, body) =
        add_over_http(&app_a, tenant_a.host(), &owner_a, &http_event).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(headers[header::RETRY_AFTER], "5");
    assert_eq!(body, json!({"error": paused[3]}));
    // A paused write must not record a media publication marker or consume an
    // uploader preview, even in shadow mode, where missing sidecars are allowed.
    let base =
        crate::api::media::media_base_url_for_tenant(&state_a.config.relay_url, tenant_a.host());
    let hash = hex::encode(Sha256::digest(Uuid::new_v4().as_bytes()));
    let create_event =
        create_migration_channel(&owner_a, Uuid::new_v4(), &format!("{base}/{hash}.png"));
    assert!(buzz_core::media::publication_hashes(&create_event, &base).contains(&hash));
    let (status, _, body) = add_over_http(&app_a, tenant_a.host(), &owner_a, &create_event).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body, json!({"error": paused[3]}));
    let marked: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM media_publications WHERE community_id = $1 AND sha256 = $2)",
    )
    .bind(tenant_a.community().as_uuid())
    .bind(&hash)
    .fetch_one(state_a.db.pool())
    .await
    .unwrap();
    assert!(!marked);
    assert!(state_b
        .db
        .get_member_role(tenant_b.community(), channel_b, recipients[0].as_bytes())
        .await
        .unwrap()
        .is_none());

    let ordinary_a = add_members(&owner_a, channel_a, &[Keys::generate().public_key()], None);
    let (status, _, body) = add_over_http(&app_a, tenant_a.host(), &owner_a, &ordinary_a).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["accepted"], true);
    let ordinary_b = add_members(&owner_b, channel_b, &[Keys::generate().public_key()], None);
    assert_eq!(send_event(&mut socket, "EVENT", &ordinary_b).await[2], true);

    assert_eq!(
        admin(&app_b, Method::PUT, Some(&operator), Some(60_000))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(send_event(&mut socket, "EVENT", &ws_event).await[2], true);
    assert!(state_b
        .db
        .get_member_role(tenant_b.community(), channel_b, recipients[0].as_bytes())
        .await
        .unwrap()
        .is_some());

    let mut unavailable = (*state_a).clone();
    unavailable.redis_pool = buzz_pubsub::redis_connection("redis://127.0.0.1:1").unwrap();
    let app = super::build_router(Arc::new(unavailable));
    assert_eq!(
        admin(&app, Method::GET, Some(&operator), None).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        admin(&app, Method::PUT, Some(&operator), Some(0)).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let tagged = add_members(
        &owner_a,
        channel_a,
        &[Keys::generate().public_key()],
        Some("slack"),
    );
    let (status, _, body) = add_over_http(&app, tenant_a.host(), &owner_a, &tagged).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(
        body,
        json!({"error": "rate-limited: shared admission unavailable", "sent": false})
    );
    let ordinary = add_members(&owner_a, channel_a, &[Keys::generate().public_key()], None);
    let (status, _, body) = add_over_http(&app, tenant_a.host(), &owner_a, &ordinary).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["accepted"], true);

    drop(socket);
    server.abort();
    let _: () = redis::cmd("DEL")
        .arg(TEST_LIMIT_KEY)
        .query_async(&mut state_a.redis_pool.clone())
        .await
        .unwrap();
}
