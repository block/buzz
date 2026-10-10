//! Real HTTP admission, command application, publication and generic discovery.
//! The fixture enables NIP-CL only here; it does not bypass transport replay checks.
#[path = "channel_labels_postgres_tests/activation_postgres_tests.rs"]
mod activation_postgres_tests;
#[path = "channel_labels_postgres_tests/repair_postgres_tests.rs"]
mod repair_postgres_tests;
#[path = "channel_labels_postgres_tests/ws_postgres_tests.rs"]
mod ws_postgres_tests;

use super::postgres_tests::bridge_handler_test_state;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use buzz_core::{channel_labels::verify_snapshot, CommunityId, TenantContext};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

struct Fixture {
    state: Arc<crate::state::AppState>,
    pool: sqlx::PgPool,
    host: String,
    community: CommunityId,
    owner: Keys,
    peer: Keys,
}

impl Fixture {
    async fn new() -> Self {
        let mut state = bridge_handler_test_state()
            .await
            .expect("Postgres and Redis required");
        let inner = Arc::get_mut(&mut state).expect("unshared fixture");
        Arc::get_mut(&mut inner.config)
            .expect("unshared config")
            .nip_cl_enabled = true;
        inner.nip98_replay = Arc::new(buzz_pubsub::RedisNip98ReplayGuard::new(
            inner.redis_pool.clone(),
        ));
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let host = format!("channel-labels-{}.local", Uuid::new_v4());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .unwrap()
            .id;
        state
            .db
            .maintain_partitions(
                1,
                buzz_db::partition::PartitionMaintenancePolicy {
                    create_enabled: true,
                    advance_enabled: false,
                },
            )
            .await
            .unwrap();
        Self {
            state,
            pool,
            host,
            community,
            owner: Keys::generate(),
            peer: Keys::generate(),
        }
    }

    fn command(&self, key: &Keys, channel: Uuid, kind: u16, fields: &[(&str, &str)]) -> Event {
        EventBuilder::new(Kind::Custom(kind), "")
            .tags(
                std::iter::once(Tag::parse(["h", &channel.to_string()]).unwrap()).chain(
                    fields
                        .iter()
                        .map(|(name, value)| Tag::parse([*name, *value]).unwrap()),
                ),
            )
            .sign_with_keys(key)
            .unwrap()
    }

    async fn request(
        &self,
        key: &Keys,
        host: &str,
        path: &str,
        body: Value,
    ) -> (axum::http::StatusCode, Value) {
        use tower::ServiceExt;
        let body = body.to_string();
        let auth = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::parse(["u", &format!("https://{host}{path}")]).unwrap(),
                Tag::parse(["method", "POST"]).unwrap(),
                Tag::parse(["payload", &hex::encode(Sha256::digest(body.as_bytes()))]).unwrap(),
                // Exact command retries still need a fresh transport proof.
                Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
            ])
            .sign_with_keys(key)
            .unwrap();
        let response = crate::router::build_router(self.state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("host", host)
                    .header("content-type", "application/json")
                    .header(
                        "authorization",
                        format!(
                            "Nostr {}",
                            BASE64.encode(serde_json::to_vec(&auth).unwrap())
                        ),
                    )
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).expect("JSON response"),
        )
    }

    async fn submit(&self, key: &Keys, event: &Event, accepted: bool, prefix: &str) {
        let (status, body) = self.request(key, &self.host, "/events", json!(event)).await;
        assert!(status.is_success(), "{status}: {body}");
        assert_eq!(body["event_id"], event.id.to_hex(), "{body}");
        assert_eq!(body["accepted"], accepted, "{body}");
        assert!(
            body["message"].as_str().unwrap().starts_with(prefix),
            "{body}"
        );
        if prefix.is_empty() {
            assert_eq!(body["message"], "");
        }
    }

    async fn read(&self, key: &Keys, path: &str, filter: Value) -> Value {
        let (status, body) = self.request(key, &self.host, path, json!([filter])).await;
        assert!(status.is_success(), "{status}: {body}");
        body
    }

    async fn snapshot(&self, channel: Uuid, labels: &[&str]) -> Event {
        let body = self
            .read(
                &self.owner,
                "/query",
                json!({"kinds":[39000],"#d":[channel]}),
            )
            .await;
        assert_eq!(body.as_array().unwrap().len(), 1, "{body}");
        let event: Event = serde_json::from_value(body[0].clone()).unwrap();
        assert_eq!(
            verify_snapshot(&event, self.state.relay_keypair.public_key(), channel)
                .unwrap()
                .values(),
            labels
        );
        event
    }

    async fn create(&self, channel: Uuid, labels: &[&str], visibility: &str) -> Event {
        let mut fields = vec![
            ("name", "label-test"),
            ("channel_type", "stream"),
            ("visibility", visibility),
        ];
        fields.extend(labels.iter().map(|label| ("label", *label)));
        let event = self.command(&self.owner, channel, 9007, &fields);
        self.submit(&self.owner, &event, true, "").await;
        event
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn router_commits_retries_and_preserves_labels_through_ordinary_publishers() {
    let f = Fixture::new().await;
    let channel = Uuid::new_v4();
    let create = f
        .create(
            channel,
            &["team:infra", "status:queued", "team:infra"],
            "open",
        )
        .await;
    let initial = f.snapshot(channel, &["status:queued", "team:infra"]).await;
    f.submit(&f.owner, &create, true, "duplicate: nip-cl-committed")
        .await;
    assert_eq!(
        f.snapshot(channel, &["status:queued", "team:infra"])
            .await
            .id,
        initial.id
    );

    let edit = f.command(
        &f.owner,
        channel,
        9002,
        &[
            ("add-label", "status:active"),
            ("remove-label", "status:queued"),
        ],
    );
    f.submit(&f.owner, &edit, true, "").await;
    let changed = f.snapshot(channel, &["status:active", "team:infra"]).await;
    assert!(changed.created_at > initial.created_at);
    let noop = f.command(&f.owner, channel, 9002, &[("add-label", "team:infra")]);
    f.submit(&f.owner, &noop, true, "").await;
    assert_eq!(
        f.snapshot(channel, &["status:active", "team:infra"])
            .await
            .id,
        changed.id
    );

    let remove = f.command(
        &f.owner,
        channel,
        9002,
        &[
            ("remove-label", "status:active"),
            ("remove-label", "team:infra"),
        ],
    );
    f.submit(&f.owner, &remove, true, "").await;
    let empty = f.snapshot(channel, &[]).await;
    assert!(empty.created_at > changed.created_at);
    for earlier in [&create, &edit, &noop] {
        f.submit(&f.owner, earlier, true, "duplicate: nip-cl-committed")
            .await;
        assert_eq!(f.snapshot(channel, &[]).await.id, empty.id);
    }
    let rename = f.command(
        &f.owner,
        channel,
        9002,
        &[("name", "renamed-without-labels")],
    );
    f.submit(&f.owner, &rename, true, "").await;
    let renamed = f.snapshot(channel, &[]).await;
    assert!(renamed.created_at > empty.created_at);
    let tenant = TenantContext::resolved(f.community, &f.host);
    crate::handlers::side_effects::emit_group_discovery_events(&tenant, &f.state, channel)
        .await
        .unwrap();
    assert_eq!(
        f.snapshot(channel, &[]).await.id,
        renamed.id,
        "repair must preserve a current head"
    );

    let applied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE community_id=$1 AND channel_id=$2 AND nip_cl_applied",
    )
    .bind(f.community.as_uuid())
    .bind(channel)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(
        applied, 4,
        "new no-op commits evidence, retries do not add commands"
    );
    let messages: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND channel_id=$2 AND kind IN (9, 40002, 40003)")
        .bind(f.community.as_uuid()).bind(channel).fetch_one(&f.pool).await.unwrap();
    assert_eq!(messages, 0, "metadata commands do not become chat activity");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn router_discovery_and_count_filter_current_authorized_heads_before_limit() {
    let f = Fixture::new().await;
    let matching = Uuid::new_v4();
    let nonmatching = Uuid::new_v4();
    let private = Uuid::new_v4();
    f.create(matching, &["team:infra"], "open").await;
    let expected = f.snapshot(matching, &["team:infra"]).await;
    for (channel, visibility) in [(nonmatching, "open"), (private, "private")] {
        f.create(channel, &["team:infra"], visibility).await;
        // Force newer heads without sleeping or assuming wall-clock ordering.
        let rename = f.command(&f.owner, channel, 9002, &[("name", "newer-head")]);
        f.submit(&f.owner, &rename, true, "").await;
        assert!(f.snapshot(channel, &["team:infra"]).await.created_at > expected.created_at);
    }
    let remove = f.command(
        &f.owner,
        nonmatching,
        9002,
        &[("remove-label", "team:infra")],
    );
    f.submit(&f.owner, &remove, true, "").await;
    // The superseded matching snapshot must not reappear after label removal.
    let filter =
        json!({"kinds":[39000],"#t":["stream"],"#L":["nip-cl"],"#l":["team:infra"],"limit":1});
    let result = f.read(&f.peer, "/query", filter.clone()).await;
    assert_eq!(result.as_array().unwrap().len(), 1, "{result}");
    assert_eq!(result[0]["id"], expected.id.to_hex());
    for mixed in [
        json!({"kinds":[39000, 1],"#L":["nip-cl"],"#l":["team:infra"],"limit":1}),
        json!({"ids":[expected.id, f.snapshot(nonmatching, &[]).await.id],"#l":["team:infra"],"limit":1}),
    ] {
        let result = f.read(&f.peer, "/query", mixed).await;
        assert_eq!(result.as_array().unwrap().len(), 1, "{result}");
        assert_eq!(result[0]["id"], expected.id.to_hex());
    }
    let count = f.read(&f.peer, "/count", filter.clone()).await;
    assert_eq!(count["count"], 1, "{count}");
    let owner_count = f.read(&f.owner, "/count", filter).await;
    assert_eq!(
        owner_count["count"], 2,
        "private fixture must exist and be readable to owner"
    );
    let missing = f
        .read(&f.peer, "/query", json!({"kinds":[39000],"#d":[private]}))
        .await;
    assert_eq!(missing, json!([]));
    let nonmatch_count = f
        .read(&f.peer, "/count", json!({"kinds":[39000],"#l":["absent"]}))
        .await;
    assert_eq!(nonmatch_count["count"], 0);

    let other_host = format!("labels-bystander-{}.local", Uuid::new_v4());
    f.state
        .db
        .ensure_configured_community(&other_host)
        .await
        .unwrap();
    let (status, result) = f
        .request(
            &f.owner,
            &other_host,
            "/query",
            json!([{"kinds":[39000],"#l":["team:infra"]}]),
        )
        .await;
    assert!(status.is_success(), "{status}: {result}");
    assert_eq!(
        result,
        json!([]),
        "host-derived community must not read neighbor labels"
    );
    let (status, result) = f
        .request(&f.owner, &other_host, "/events", json!(remove))
        .await;
    assert!(status.is_success(), "{status}: {result}");
    assert_eq!(result["accepted"], false);
    assert!(result["message"]
        .as_str()
        .unwrap()
        .starts_with("restricted: nip-cl-rejected"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn router_rejections_leave_no_partial_state_and_unknown_is_not_a_receipt() {
    let f = Fixture::new().await;
    let invalid = Uuid::new_v4();
    let create = f.command(
        &f.owner,
        invalid,
        9007,
        &[("name", "invalid-label"), ("label", "Uppercase")],
    );
    f.submit(&f.owner, &create, false, "invalid: nip-cl-rejected")
        .await;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM channels WHERE community_id=$1 AND id=$2")
            .bind(f.community.as_uuid())
            .bind(invalid)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    let channel = Uuid::new_v4();
    let committed = f.create(channel, &[], "open").await;
    let before = f.snapshot(channel, &[]).await;
    let conflict = f.command(&f.owner, channel, 9007, &[("name", "different-command")]);
    f.submit(&f.owner, &conflict, false, "duplicate: nip-cl-rejected")
        .await;
    let unauthorized = f.command(&f.peer, channel, 9002, &[("add-label", "not-authorized")]);
    f.submit(&f.peer, &unauthorized, false, "restricted: nip-cl-rejected")
        .await;
    let mixed = f.command(
        &f.owner,
        channel,
        9002,
        &[("name", "not-applied"), ("add-label", "mixed")],
    );
    f.submit(&f.owner, &mixed, false, "invalid: nip-cl-rejected")
        .await;
    assert_eq!(f.snapshot(channel, &[]).await.id, before.id);

    let legacy = f.command(&f.owner, channel, 9002, &[("add-label", "legacy")]);
    f.state
        .db
        .insert_event(f.community, &legacy, Some(channel))
        .await
        .unwrap();
    f.submit(&f.owner, &legacy, false, "error: nip-cl-unknown")
        .await;
    assert_eq!(f.snapshot(channel, &[]).await.id, before.id);
    // Soft deletion cannot make a previously committed command apply again.
    sqlx::query("UPDATE events SET deleted_at=NOW() WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(committed.id.as_bytes().as_slice())
        .execute(&f.pool)
        .await
        .unwrap();
    f.submit(&f.owner, &committed, true, "duplicate: nip-cl-committed")
        .await;
    f.state
        .db
        .archive_channel(f.community, channel)
        .await
        .unwrap();
    f.submit(&f.owner, &committed, false, "restricted: nip-cl-rejected")
        .await;
    assert_eq!(f.snapshot(channel, &[]).await.id, before.id);
}
