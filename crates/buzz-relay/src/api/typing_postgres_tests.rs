// Typing indicators (kind:20002) submitted through HTTP POST /events.
use super::postgres_tests::bridge_handler_test_state;
use super::*;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use uuid::Uuid;

struct Fixture {
    state: Arc<crate::state::AppState>,
    pool: sqlx::PgPool,
    host: String,
    community: buzz_core::CommunityId,
    channel: Uuid,
    member: Keys,
}

impl Fixture {
    async fn new() -> Self {
        let state = bridge_handler_test_state()
            .await
            .expect("Postgres and Redis");
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let host = format!("typing-{}.local", Uuid::new_v4());
        state.db.ensure_configured_community(&host).await.unwrap();
        let community = crate::tenant::bind_community(&state.db, &host)
            .await
            .unwrap()
            .community();
        let channel = Uuid::new_v4();
        let member = Keys::generate();
        sqlx::query("INSERT INTO channels(community_id,id,name,visibility,channel_type,created_by) VALUES($1,$2,$2::text,'private'::channel_visibility,'stream',$3)")
            .bind(community.as_uuid()).bind(channel).bind(member.public_key().as_bytes().as_slice()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey,role) VALUES($1,$2,$3,'owner')")
            .bind(community.as_uuid()).bind(channel).bind(member.public_key().as_bytes().as_slice()).execute(&pool).await.unwrap();
        Self {
            state,
            pool,
            host,
            community,
            channel,
            member,
        }
    }

    fn typing(&self, key: &Keys, channel: Option<Uuid>) -> Event {
        let tags = channel.map(|ch| Tag::parse(["h".to_string(), ch.to_string()]).unwrap());
        EventBuilder::new(Kind::Custom(20002), "")
            .tags(tags)
            .sign_with_keys(key)
            .unwrap()
    }

    async fn post(&self, auth: &Keys, event: &Event) -> (axum::http::StatusCode, Value) {
        use tower::ServiceExt;
        let response = crate::router::build_router(self.state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/events")
                    .header("host", &self.host)
                    .header("content-type", "application/json")
                    .header("x-pubkey", auth.public_key().to_hex())
                    .body(axum::body::Body::from(json!(event).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// Subscribes to the channel's Redis topic; acknowledged before returning,
    /// so a later publish is not missed.
    async fn subscribe_channel(&self) -> redis::aio::PubSub {
        let topic = buzz_pubsub::EventTopicKey {
            community_id: self.community,
            topic: buzz_pubsub::EventTopic::Channel(self.channel),
        }
        .redis_channel();
        let client = redis::Client::open(self.state.config.redis_url.as_str()).unwrap();
        let mut pubsub = client.get_async_pubsub().await.unwrap();
        pubsub.subscribe(&topic).await.unwrap();
        pubsub
    }

    async fn stored_typing_rows(&self) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM events WHERE community_id = $1 AND channel_id = $2 AND kind = 20002",
        )
        .bind(self.community.as_uuid())
        .bind(self.channel)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn member_typing_is_broadcast_and_not_stored() {
    use futures::StreamExt;
    let f = Fixture::new().await;
    let pubsub = f.subscribe_channel().await;

    let event = f.typing(&f.member, Some(f.channel));
    let (status, body) = f.post(&f.member, &event).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert_eq!(body["accepted"], true, "{body}");
    assert_eq!(body["event_id"], event.id.to_hex(), "{body}");

    let mut messages = pubsub.into_on_message();
    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), messages.next())
        .await
        .expect("typing indicator published to the channel topic")
        .unwrap();
    let published: Event = serde_json::from_str(&msg.get_payload::<String>().unwrap()).unwrap();
    assert_eq!(published.id, event.id);
    assert_eq!(f.stored_typing_rows().await, 0);
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn non_member_typing_is_rejected() {
    let f = Fixture::new().await;
    let outsider = Keys::generate();
    let (status, body) = f
        .post(&outsider, &f.typing(&outsider, Some(f.channel)))
        .await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "restricted: not a channel member", "{body}");
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn timed_out_member_typing_is_rejected() {
    let f = Fixture::new().await;
    f.state
        .db
        .timeout_community_member(
            f.community,
            f.member.public_key().as_bytes(),
            Keys::generate().public_key().as_bytes(),
            chrono::Utc::now() + chrono::Duration::hours(1),
            None,
        )
        .await
        .unwrap();
    let (status, body) = f
        .post(&f.member, &f.typing(&f.member, Some(f.channel)))
        .await;
    assert_eq!(status, axum::http::StatusCode::FORBIDDEN, "{body}");
}

/// HTTP typing reads the same short-TTL restriction cache as the WebSocket
/// ephemeral path: a cached ban refuses a member Postgres never restricted.
/// Mutation: call the uncached `enforce_write_restriction` → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn typing_uses_cached_restriction_row() {
    let f = Fixture::new().await;
    f.state.restriction_cache.insert(
        (f.community, f.member.public_key().to_bytes().to_vec()),
        buzz_db::moderation::RestrictionState {
            banned: true,
            muted_until: None,
        },
    );
    let (status, body) = f
        .post(&f.member, &f.typing(&f.member, Some(f.channel)))
        .await;
    assert_eq!(status, axum::http::StatusCode::FORBIDDEN, "{body}");
}

/// The membership-stage ban gate for HTTP typing reads the same cache: a
/// cached clear row admits a member Postgres banned within the TTL.
/// Mutation: route typing through `enforce_relay_membership` (fresh ban
/// read) → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn typing_ban_gate_uses_cached_restriction_row() {
    let f = Fixture::new().await;
    let member = f.member.public_key().to_bytes().to_vec();
    f.state
        .db
        .ban_community_member(
            f.community,
            &member,
            Keys::generate().public_key().as_bytes(),
            None,
            None,
        )
        .await
        .unwrap();
    f.state.restriction_cache.insert(
        (f.community, member),
        buzz_db::moderation::RestrictionState {
            banned: false,
            muted_until: None,
        },
    );
    let (status, body) = f
        .post(&f.member, &f.typing(&f.member, Some(f.channel)))
        .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
}

/// Only typing uses the cached ban gate: a persistent post reads ban state
/// fresh, so a stale cached ban does not refuse a member Postgres cleared.
/// Mutation: use the cached ban gate for every kind → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn persistent_post_ban_gate_reads_fresh() {
    let f = Fixture::new().await;
    f.state.restriction_cache.insert(
        (f.community, f.member.public_key().to_bytes().to_vec()),
        buzz_db::moderation::RestrictionState {
            banned: true,
            muted_until: None,
        },
    );
    let message = EventBuilder::new(Kind::Custom(9), "hello")
        .tags([Tag::parse(["h".to_string(), f.channel.to_string()]).unwrap()])
        .sign_with_keys(&f.member)
        .unwrap();
    let (status, body) = f.post(&f.member, &message).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert_eq!(body["accepted"], true, "{body}");
}

/// HTTP typing reads the cached serving state, like the WebSocket path.
/// Mutation: call the uncached `is_serving_active` → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn typing_uses_cached_serving_state() {
    let f = Fixture::new().await;
    f.state.serving_active_cache.insert(f.community, false);
    let (status, body) = f
        .post(&f.member, &f.typing(&f.member, Some(f.channel)))
        .await;
    assert_ne!(status, axum::http::StatusCode::OK, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("community writes are fenced"),
        "{body}"
    );
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn typing_signed_by_someone_else_is_rejected() {
    let f = Fixture::new().await;
    let (status, body) = f
        .post(&Keys::generate(), &f.typing(&f.member, Some(f.channel)))
        .await;
    assert_eq!(status, axum::http::StatusCode::FORBIDDEN, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("does not match authenticated identity"),
        "{body}"
    );
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn tampered_typing_is_rejected() {
    let f = Fixture::new().await;
    let mut raw = serde_json::to_value(f.typing(&f.member, Some(f.channel))).unwrap();
    raw["content"] = json!("tampered");
    let event: Event = serde_json::from_value(raw).unwrap();
    let (status, body) = f.post(&f.member, &event).await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(f.stored_typing_rows().await, 0);
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn other_ephemeral_kinds_are_still_rejected() {
    let f = Fixture::new().await;
    let event = EventBuilder::new(Kind::Custom(20001), "")
        .tags([Tag::parse(["h".to_string(), f.channel.to_string()]).unwrap()])
        .sign_with_keys(&f.member)
        .unwrap();
    let (status, body) = f.post(&f.member, &event).await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn typing_without_channel_is_rejected() {
    let f = Fixture::new().await;
    let (status, body) = f.post(&f.member, &f.typing(&f.member, None)).await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().unwrap_or_default().contains("h tag"),
        "{body}"
    );
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn oversized_typing_is_rejected() {
    let f = Fixture::new().await;
    let h = || Tag::parse(["h".to_string(), f.channel.to_string()]).unwrap();
    let empty = || Tag::parse([String::new()]).unwrap();
    let cases = [
        ("content", "x".repeat(2048), vec![h()]),
        (
            "tag value",
            String::new(),
            vec![
                h(),
                Tag::parse(["e".to_string(), "x".repeat(2048)]).unwrap(),
            ],
        ),
        (
            "empty tags",
            String::new(),
            std::iter::once(h())
                .chain(std::iter::repeat_with(empty).take(10_000))
                .collect(),
        ),
    ];
    let pubsub = f.subscribe_channel().await;
    for (case, content, tags) in cases {
        let event = EventBuilder::new(Kind::Custom(20002), content)
            .tags(tags)
            .sign_with_keys(&f.member)
            .unwrap();
        let (status, body) = f.post(&f.member, &event).await;
        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "{case}: {body}"
        );
        assert!(
            body["error"]
                .as_str()
                .unwrap_or_default()
                .contains("too large"),
            "{case}: {body}"
        );
    }
    assert_nothing_published(pubsub, "oversized typing indicator").await;
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn fenced_community_typing_is_rejected() {
    let f = Fixture::new().await;
    // Over HTTP a quiescing community's host no longer binds (404), so the
    // fence only matters when quiescing starts after the request has bound its
    // tenant. Reproduce that order: bind first, then quiesce, then publish.
    let tenant = crate::tenant::bind_community(&f.state.db, &f.host)
        .await
        .unwrap();
    // Enter the deletion executor's transaction scope in this disposable
    // fixture; the DB rejects unfenced ad-hoc state changes.
    let mut tx = f.pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('buzz.deletion_executor_community', $1, true), set_config('buzz.deletion_fence_generation', '0', true)")
        .bind(f.community.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE communities SET deletion_state = 'quiescing' WHERE id = $1")
        .bind(f.community.as_uuid())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let pubsub = f.subscribe_channel().await;

    let err = crate::handlers::event::publish_http_typing(
        &f.state,
        &tenant,
        f.typing(&f.member, Some(f.channel)),
        f.member.public_key(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, IngestError::Rejected(msg) if msg == "restricted: community writes are fenced"),
        "{err:?}"
    );
    assert_nothing_published(pubsub, "fenced typing indicator").await;
}

async fn assert_nothing_published(pubsub: redis::aio::PubSub, what: &str) {
    use futures::StreamExt;
    let mut messages = pubsub.into_on_message();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(500), messages.next())
            .await
            .is_err(),
        "{what} must not be published"
    );
}
