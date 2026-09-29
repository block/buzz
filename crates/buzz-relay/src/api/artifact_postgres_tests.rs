// Exercises the production HTTP router and ingest gates for artifacts.
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
    home: Uuid,
    private: Uuid,
    owner: Keys,
    peer: Keys,
}
impl Fixture {
    async fn new() -> Self {
        let state = bridge_handler_test_state()
            .await
            .expect("Postgres and Redis");
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let host = format!("artifact-review-{}.local", Uuid::new_v4());
        state.db.ensure_configured_community(&host).await.unwrap();
        state.db.ensure_future_partitions(1).await.unwrap();
        let tenant = crate::tenant::bind_community(&state.db, &host)
            .await
            .unwrap();
        let community = tenant.community();
        let home = Uuid::new_v4();
        let private = Uuid::new_v4();
        let owner = Keys::generate();
        for (id, visibility) in [(home, "open"), (private, "private")] {
            sqlx::query("INSERT INTO channels(community_id,id,name,visibility,channel_type,created_by) VALUES($1,$2,$2::text,$3::channel_visibility,'stream',$4)")
                .bind(community.as_uuid()).bind(id).bind(visibility).bind(owner.public_key().as_bytes().as_slice()).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey,role) VALUES($1,$2,$3,'owner')")
                .bind(community.as_uuid()).bind(id).bind(owner.public_key().as_bytes().as_slice()).execute(&pool).await.unwrap();
        }
        Self {
            state,
            pool,
            host,
            community,
            home,
            private,
            owner,
            peer: Keys::generate(),
        }
    }
    fn revision(&self, id: Uuid, op: &str, prev: Option<&Event>) -> Event {
        self.revision_as(&self.owner, self.home, id, op, prev)
    }
    fn revision_as(
        &self,
        key: &Keys,
        home: Uuid,
        id: Uuid,
        op: &str,
        prev: Option<&Event>,
    ) -> Event {
        let mut tags = vec![
            vec!["ar".into(), "1".into()],
            vec!["d".into(), id.to_string()],
            vec!["h".into(), home.to_string()],
            vec!["type".into(), "buzz.task".into()],
            vec!["op".into(), op.into()],
        ];
        if op != "delete" {
            tags.push(vec!["title".into(), "Review task".into()]);
        }
        if let Some(prev) = prev {
            tags.push(vec!["prev".into(), prev.id.to_hex()]);
        }
        EventBuilder::new(
            Kind::Custom(45010),
            if op == "delete" {
                ""
            } else {
                "review searchable"
            },
        )
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .sign_with_keys(key)
        .unwrap()
    }
    async fn request(&self, path: &str, body: Value) -> (axum::http::StatusCode, Value) {
        self.request_as(&self.owner, path, body).await
    }
    async fn request_as(
        &self,
        key: &Keys,
        path: &str,
        body: Value,
    ) -> (axum::http::StatusCode, Value) {
        use tower::ServiceExt;
        let response = crate::router::build_router(self.state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("host", &self.host)
                    .header("content-type", "application/json")
                    .header("x-pubkey", key.public_key().to_hex())
                    .body(axum::body::Body::from(body.to_string()))
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
    async fn publish(&self, event: &Event) {
        let (status, body) = self.try_publish(event).await;
        assert!(status.is_success(), "{status}: {body}");
        assert_eq!(body["accepted"], true, "{body}");
    }
    async fn try_publish(&self, event: &Event) -> (axum::http::StatusCode, Value) {
        let key = if event.pubkey == self.peer.public_key() {
            &self.peer
        } else {
            &self.owner
        };
        self.request_as(key, "/events", json!(event)).await
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn project_filter_survives_artifact_query_hook() {
    let f = Fixture::new().await;
    let project = EventBuilder::new(Kind::Custom(30621), "")
        .tags([
            Tag::parse(["d", "review-project"]).unwrap(),
            Tag::parse(["buzz-channel", &f.home.to_string()]).unwrap(),
        ])
        .sign_with_keys(&f.owner)
        .unwrap();
    f.state
        .db
        .insert_event(f.community, &project, None)
        .await
        .unwrap();
    let filter = json!([{"kinds":[30621],"#buzz-channel":[f.home]}]);
    let (status, body) = f.request("/query", filter.clone()).await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["id"], project.id.to_hex());
    let (status, body) = f.request("/count", filter).await;
    assert!(status.is_success(), "{status}: {body}");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn unsupported_predicates_fail_explicitly() {
    let f = Fixture::new().await;
    let create = f.revision(Uuid::new_v4(), "create", None);
    f.publish(&create).await;
    let (status, body) = f.request("/query", json!([{"ids":[create.id]}])).await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(body[0]["id"], create.id.to_hex());
    // A readable artifact must not match a predicate it does not carry.
    for path in ["/query", "/count"] {
        for filter in [
            json!({"ids":[create.id],"#project":["P"]}),
            json!({"artifact":"current","#assignee":[]}),
        ] {
            let (status, body) = f.request(path, json!([filter])).await;
            assert_eq!(
                status,
                axum::http::StatusCode::BAD_REQUEST,
                "{filter}: {body}"
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn lifecycle_routes_duplicate_replay_delete_and_redaction() {
    let f = Fixture::new().await;
    let d = Uuid::new_v4();
    let create = f.revision(d, "create", None);
    f.publish(&create).await;
    let update = f.revision(d, "update", Some(&create));
    f.publish(&update).await;
    f.publish(&create).await;
    // The production ingest duplicate path must precede timestamp freshness.
    let old = EventBuilder::new(Kind::Custom(45010), "old")
        .tags(create.tags.iter().cloned())
        .custom_created_at(nostr::Timestamp::from(1))
        .sign_with_keys(&f.owner)
        .unwrap();
    sqlx::query(
        "INSERT INTO artifact_revisions(community_id,event_id,artifact_id) VALUES($1,$2,$3)",
    )
    .bind(f.community.as_uuid())
    .bind(old.id.as_bytes().as_slice())
    .bind(Uuid::new_v4())
    .execute(&f.pool)
    .await
    .unwrap();
    f.publish(&old).await;
    let delete = f.revision(d, "delete", Some(&update));
    f.publish(&delete).await;
    // WebSocket replay's production filter-to-query and store seams.
    let filter = serde_json::from_value(json!({"kinds":[45010],"#h":[f.home],"since":0})).unwrap();
    let query = crate::handlers::req::build_event_query_from_filter(
        &filter,
        f.owner.public_key().as_bytes(),
        &f.state,
        f.community,
    )
    .await;
    let replay = f.state.db.query_events(&query).await.unwrap();
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].event.id, delete.id);
    for path in ["/query", "/count"] {
        let (status, body) = f
            .request(path, json!([{"artifact":"current","#d":[d]}]))
            .await;
        assert!(status.is_success(), "{body}");
        if path == "/query" {
            assert_eq!(body, json!([]));
        } else {
            assert_eq!(body["count"], 0);
        }
    }
    // Multi-character predicates are never silently dropped on the generic path.
    let (status, _) = f
        .request("/query", json!([{"kinds":[45010],"#project":["p"]}]))
        .await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    let (_, search) = f
        .request(
            "/query",
            json!([{"kinds":[9],"search":"review searchable","#h":[f.home]}]),
        )
        .await;
    assert_eq!(search, json!([]));
    let restore = f.revision(d, "restore", Some(&delete));
    f.publish(&restore).await;
    let command = |kind| {
        EventBuilder::new(Kind::Custom(kind), "")
            .tags([
                Tag::parse(["e", &restore.id.to_hex()]).unwrap(),
                Tag::parse(["h", &f.home.to_string()]).unwrap(),
            ])
            .sign_with_keys(&f.owner)
            .unwrap()
    };
    let (status, body) = f.request("/events", json!(command(5))).await;
    assert!(!status.is_success(), "{body}");
    assert!(
        body.to_string()
            .contains("artifacts cannot be deleted with kind 5"),
        "{body}"
    );
    // Redaction is the ordinary 9005 removal; the head stays editable.
    f.publish(&command(9005)).await;
    let current = json!([{"artifact":"current","#d":[d]}]);
    let (_, body) = f.request("/query", current.clone()).await;
    assert_eq!(body, json!([]));
    let (_, body) = f
        .request("/count", json!([{"artifact":"history","#d":[d]}]))
        .await;
    assert_eq!(body["count"], 3);
    let edit = f.revision(d, "update", Some(&restore));
    f.publish(&edit).await;
    let (_, body) = f.request("/query", current).await;
    assert_eq!(body[0]["id"], edit.id.to_hex());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn writes_use_channel_gates_for_home_and_move_source() {
    let f = Fixture::new().await;
    let d = Uuid::new_v4();
    let create = f.revision(d, "create", None);
    f.publish(&create).await;
    // Open-channel nonmembers may edit, as they may post kind 9.
    let peer_edit = f.revision_as(&f.peer, f.home, d, "update", Some(&create));
    f.publish(&peer_edit).await;
    let other = f.revision_as(&f.peer, f.private, Uuid::new_v4(), "create", None);
    let (status, body) = f.try_publish(&other).await;
    assert!(!status.is_success() || body["accepted"] == false, "{body}");
    let moved = f.revision_as(&f.owner, f.private, d, "move", Some(&peer_edit));
    f.publish(&moved).await;
    let (_, removals) = f
        .request("/query", json!([{"kinds":[45011],"#h":[f.home]}]))
        .await;
    assert_eq!(removals.as_array().unwrap().len(), 1, "{removals}");
    assert!(!removals.to_string().contains(&f.private.to_string()));
    // The marker names only the source-readable revision it replaced.
    assert!(removals[0]["tags"]
        .as_array()
        .unwrap()
        .contains(&json!(["prev", peer_edit.id.to_hex()])));
    // The peer can write the destination but not the private source.
    let back = f.revision_as(&f.peer, f.home, d, "move", Some(&moved));
    let (status, body) = f.try_publish(&back).await;
    assert!(!status.is_success() || body["accepted"] == false, "{body}");
    assert!(body.to_string().contains("not a channel member"), "{body}");
    sqlx::query("UPDATE channels SET archived_at=now() WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(f.private)
        .execute(&f.pool)
        .await
        .unwrap();
    let archived_source = f.revision_as(&f.owner, f.home, d, "move", Some(&moved));
    let (status, body) = f.try_publish(&archived_source).await;
    assert!(!status.is_success() || body["accepted"] == false, "{body}");
    assert!(body.to_string().contains("archived"), "{body}");
}

/// Re-sign an artifact revision with extra tags and an explicit timestamp.
fn resign(event: &Event, key: &Keys, extra: &[[&str; 2]], created_at: u64) -> Event {
    EventBuilder::new(event.kind, event.content.clone())
        .tags(
            event
                .tags
                .iter()
                .cloned()
                .chain(extra.iter().map(|t| Tag::parse(*t).unwrap())),
        )
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(key)
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn generic_p_reads_find_mentioned_artifacts() {
    let f = Fixture::new().await;
    let tagged = f.peer.public_key().to_hex();
    let create = f.revision(Uuid::new_v4(), "create", None);
    let create = resign(
        &create,
        &f.owner,
        &[["p", &tagged]],
        create.created_at.as_secs(),
    );
    f.publish(&create).await;
    let filter = json!([{"kinds":[45010],"#h":[f.home],"#p":[tagged]}]);
    let (status, body) = f.request("/query", filter.clone()).await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(body.as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body[0]["id"], create.id.to_hex());
    let (status, body) = f.request("/count", filter).await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(body["count"], 1, "{body}");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn generic_d_lookup_matches_before_limit() {
    let f = Fixture::new().await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let now = nostr::Timestamp::now().as_secs();
    // Kindless reads are p-gated to the reader, so every row tags the peer.
    let reader = f.peer.public_key().to_hex();
    let tag_reader = [["p", reader.as_str()]];
    let older = resign(
        &f.revision(a, "create", None),
        &f.owner,
        &tag_reader,
        now - 60,
    );
    f.publish(&older).await;
    let newer = resign(&f.revision(b, "create", None), &f.owner, &tag_reader, now);
    f.publish(&newer).await;
    for filter in [
        json!([{"kinds":[45010],"#h":[f.home],"#d":[a],"limit":1}]),
        json!([{"kinds":[45010,45011],"#h":[f.home],"#d":[a],"limit":1}]),
        json!([{"#h":[f.home],"#p":[reader],"#d":[a],"limit":1}]),
    ] {
        let (status, body) = f.request_as(&f.peer, "/query", filter.clone()).await;
        assert!(status.is_success(), "{status}: {body}");
        assert_eq!(body.as_array().unwrap().len(), 1, "{filter}: {body}");
        assert_eq!(body[0]["id"], older.id.to_hex(), "{filter}");
        let (status, body) = f.request_as(&f.peer, "/count", filter.clone()).await;
        assert!(status.is_success(), "{status}: {body}");
        assert_eq!(body["count"], 1, "{filter}: {body}");
    }

    // Non-artifact rows still reach the generic `#d` post-filter.
    let message = EventBuilder::new(Kind::Custom(9), "d-tagged chat")
        .tags([
            Tag::parse(["h", &f.home.to_string()]).unwrap(),
            Tag::parse(["d", &a.to_string()]).unwrap(),
            Tag::parse(["p", &reader]).unwrap(),
        ])
        .sign_with_keys(&f.owner)
        .unwrap();
    f.publish(&message).await;
    let filter = json!([{"#h":[f.home],"#p":[reader],"#d":[a]}]);
    let (status, body) = f.request_as(&f.peer, "/query", filter).await;
    assert!(status.is_success(), "{status}: {body}");
    let mut ids: Vec<_> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    let mut expected = vec![older.id.to_hex(), message.id.to_hex()];
    expected.sort();
    assert_eq!(ids, expected, "{body}");
}
