use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request as HttpRequest,
};
use base64::Engine;
use buzz_core::{
    channel::{ChannelType, ChannelVisibility},
    CommunityId,
};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    state: Arc<AppState>,
    host: String,
    community: CommunityId,
    channel: Uuid,
    keys: Keys,
    root: Event,
    pool: sqlx::PgPool,
}

impl Fixture {
    async fn new() -> Self {
        let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
            .await
            .unwrap();
        let mut state = (*state).clone();
        // Use production after_connect policy (floor guard, isolation and
        // session timeouts), not raw SQLx pools that mask deployed failures.
        state.db = buzz_db::Db::new(&buzz_db::DbConfig {
            database_url: crate::test_support::database_url(),
            max_connections: 5,
            ..Default::default()
        })
        .await
        .unwrap();
        Arc::make_mut(&mut state.config).require_auth_token = true;
        state.nip98_replay = Arc::new(buzz_pubsub::RedisNip98ReplayGuard::new(
            state.redis_pool.clone(),
        ));
        let state = Arc::new(state);
        let host = format!("tw-{}.local", Uuid::new_v4());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .unwrap()
            .id;
        let channel = Uuid::new_v4();
        let keys = Keys::generate();
        state
            .db
            .create_channel_with_id(
                community,
                channel,
                "thread-window",
                ChannelType::Stream,
                ChannelVisibility::Private,
                None,
                &keys.public_key().to_bytes(),
                None,
            )
            .await
            .unwrap();
        let root = event(&keys, channel, 9, "root", None, Timestamp::now().as_secs());
        state
            .db
            .insert_event(community, &root, Some(channel))
            .await
            .unwrap();
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        Self {
            state,
            host,
            community,
            channel,
            keys,
            root,
            pool,
        }
    }
    fn filter(&self) -> Value {
        json!({"thread_window":true,"#h":[self.channel],"#e":[self.root.id.to_hex()],
            "kinds":[9],"limit":50,"include_aux":true})
    }
    async fn post(&self, key: &Keys, path: &str, body: Value) -> (StatusCode, Value) {
        post(self.state.clone(), &self.host, key, path, body).await
    }
    async fn reply(&self, n: usize) -> Event {
        let reply = event(
            &self.keys,
            self.channel,
            9,
            &format!("reply {n}"),
            Some(&self.root),
            self.root.created_at.as_secs() + n as u64,
        );
        let ts = chrono::DateTime::from_timestamp(reply.created_at.as_secs() as i64, 0).unwrap();
        let root_ts =
            chrono::DateTime::from_timestamp(self.root.created_at.as_secs() as i64, 0).unwrap();
        self.state
            .db
            .insert_event_with_thread_metadata(
                self.community,
                &reply,
                Some(self.channel),
                Some(buzz_db::event::ThreadMetadataParams {
                    event_id: &reply.id.to_bytes(),
                    event_created_at: ts,
                    channel_id: self.channel,
                    parent_event_id: Some(&self.root.id.to_bytes()),
                    parent_event_created_at: Some(root_ts),
                    root_event_id: Some(&self.root.id.to_bytes()),
                    root_event_created_at: Some(root_ts),
                    depth: 1,
                    broadcast: false,
                }),
            )
            .await
            .unwrap();
        reply
    }
    async fn aux(&self, kind: u16, target: &Event, channel: Option<Uuid>) -> Event {
        let aux = event(
            &self.keys,
            self.channel,
            kind,
            &format!("aux {}", Uuid::new_v4()),
            Some(target),
            self.root.created_at.as_secs() + 60,
        );
        self.state
            .db
            .insert_event(self.community, &aux, channel)
            .await
            .unwrap();
        aux
    }
    fn bounds(&self, response: &Value, filter: &Value) -> Value {
        let bounds = response
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["kind"] == 39007)
            .collect::<Vec<_>>();
        assert_eq!(bounds.len(), 1, "{response}");
        let event: Event = serde_json::from_value(bounds[0].clone()).unwrap();
        event.verify().unwrap();
        assert_eq!(event.pubkey, self.state.relay_keypair.public_key());
        let tags: Value = serde_json::to_value(&event.tags).unwrap();
        assert_eq!(
            tags,
            json!([
                [
                    "d",
                    Request::parse(filter)
                        .unwrap()
                        .binding(&self.host, &self.keys.public_key().to_hex())
                ],
                ["h", self.channel.to_string()],
                ["e", self.root.id.to_hex()]
            ])
        );
        serde_json::from_str(&event.content).unwrap()
    }
}

fn event(
    keys: &Keys,
    channel: Uuid,
    kind: u16,
    content: &str,
    target: Option<&Event>,
    ts: u64,
) -> Event {
    let mut tags = vec![Tag::parse(["h", &channel.to_string()]).unwrap()];
    if let Some(target) = target {
        tags.push(Tag::parse(["e", &target.id.to_hex(), "", "reply"]).unwrap());
    }
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .custom_created_at(Timestamp::from(ts))
        .sign_with_keys(keys)
        .unwrap()
}

async fn post(
    state: Arc<AppState>,
    host: &str,
    keys: &Keys,
    path: &str,
    value: Value,
) -> (StatusCode, Value) {
    let body = serde_json::to_vec(&value).unwrap();
    let proof = EventBuilder::new(Kind::Custom(27235), "")
        .tags([
            Tag::parse(["u", &format!("https://{host}{path}")]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
            Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    let auth = format!(
        "Nostr {}",
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&proof).unwrap())
    );
    let response = crate::router::build_router(state)
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri(path)
                .header("host", host)
                .header("authorization", auth)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_real_query_signed_bounds_and_sentinel_aux() {
    let f = Fixture::new().await;
    let filter = f.filter();
    let (status, empty) = f.post(&f.keys, "/query", json!([filter])).await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(f.bounds(&empty, &filter)["has_more"], false);
    let mut replies = Vec::new();
    for n in 0..51 {
        replies.push(f.reply(n).await);
    }
    let sentinel_aux = f.aux(7, &replies[0], Some(f.channel)).await;
    let reaction = f.aux(7, &replies[50], Some(f.channel)).await;
    let deletion = f.aux(5, &reaction, None).await;
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(reaction.id.to_bytes().to_vec())
        .execute(&f.pool)
        .await
        .unwrap();
    let root_edit = f.aux(40003, &f.root, Some(f.channel)).await;
    let (status, page) = f.post(&f.keys, "/query", json!([filter])).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let rows = page
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == 9)
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 50);
    assert_eq!(rows[0]["id"], replies[50].id.to_hex());
    assert_eq!(rows[49]["id"], replies[1].id.to_hex());
    let ids = page
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(!ids.contains(&sentinel_aux.id.to_hex().as_str()));
    assert!(!ids.contains(&reaction.id.to_hex().as_str()));
    assert!(ids.contains(&deletion.id.to_hex().as_str()));
    assert!(ids.contains(&root_edit.id.to_hex().as_str()));
    let bounds = f.bounds(&page, &filter);
    assert_eq!(bounds["has_more"], true);
    assert_eq!(bounds["next_cursor"]["id"], replies[1].id.to_hex());
    let mut next = filter.clone();
    next["until"] = bounds["next_cursor"]["created_at"].clone();
    next["before_id"] = bounds["next_cursor"]["id"].clone();
    let (status, tail) = f.post(&f.keys, "/query", json!([next])).await;
    assert_eq!(status, StatusCode::OK, "{tail}");
    assert_eq!(f.bounds(&tail, &next)["next_cursor"], Value::Null);
    assert_eq!(
        tail.as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == 9)
            .count(),
        1
    );
    // Actual mobile sentinel + unbounded depth remain valid ONLY in legacy.
    // Both cursor spellings and absent/false opt-in preserve ASC/ASC and no bounds.
    for flag in [None, Some(false)] {
        for (ts_key, id_key) in [
            ("thread_cursor", "thread_cursor_id"),
            ("threadCursor", "threadCursorId"),
        ] {
            let mut legacy = json!({"#h":[f.channel],"#e":[f.root.id.to_hex()],"kinds":[9],
                "depth_limit":2147483647,"limit":2});
            legacy[ts_key] = json!(-1);
            if let Some(flag) = flag {
                legacy["thread_window"] = json!(flag);
            }
            let (status, old) = f.post(&f.keys, "/query", json!([legacy])).await;
            assert_eq!(status, StatusCode::OK, "{old}");
            assert_eq!(old[0]["id"], replies[0].id.to_hex());
            assert_eq!(old[1]["id"], replies[1].id.to_hex());
            assert_eq!(old.as_array().unwrap().len(), 2);
            legacy[ts_key] = json!(replies[1].created_at.as_secs());
            legacy[id_key] = json!(replies[1].id.to_hex());
            let (status, next) = f.post(&f.keys, "/query", json!([legacy])).await;
            assert_eq!(status, StatusCode::OK, "{next}");
            assert_eq!(next[0]["id"], replies[2].id.to_hex());
            assert_eq!(next[1]["id"], replies[3].id.to_hex());
            assert_eq!(next.as_array().unwrap().len(), 2);
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_real_query_denial_revocation_and_colliding_tenants() {
    let f = Fixture::new().await;
    f.reply(0).await;
    let filter = f.filter();
    let outsider = Keys::generate();
    let (status, denied) = f.post(&outsider, "/query", json!([filter])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(denied, json!([]));
    let other_host = format!("other-{}.local", Uuid::new_v4());
    let other = f
        .state
        .db
        .ensure_configured_community(&other_host)
        .await
        .unwrap()
        .id;
    f.state
        .db
        .create_channel_with_id(
            other,
            f.channel,
            "same-channel",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            &f.keys.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap();
    // Same channel ID, same accessible reader, absent root in the other tenant.
    let (status, other_page) = post(
        f.state.clone(),
        &other_host,
        &f.keys,
        "/query",
        json!([filter]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(other_page.as_array().unwrap().len(), 1);
    let other_bounds: Event = serde_json::from_value(other_page[0].clone()).unwrap();
    other_bounds.verify().unwrap();
    assert_eq!(other_bounds.pubkey, f.state.relay_keypair.public_key());
    assert!(other_bounds.tags.iter().any(|t| t.as_slice()
        == [
            "d",
            &Request::parse(&filter)
                .unwrap()
                .binding(&other_host, &f.keys.public_key().to_hex())
        ]));
    assert_eq!(
        serde_json::from_str::<Value>(&other_bounds.content).unwrap()["has_more"],
        false
    );
    let (status, _) = f.post(&f.keys, "/query", json!([filter])).await;
    assert_eq!(status, StatusCode::OK);
    // Prime the usual cached access, then revoke directly on the writer without
    // cache invalidation (the shape of cross-node delayed invalidation).
    assert!(f
        .state
        .get_accessible_channel_ids_cached(f.community, &f.keys.public_key().to_bytes())
        .await
        .unwrap()
        .contains(&f.channel));
    sqlx::query(
        "UPDATE channel_members SET removed_at=now() WHERE community_id=$1 AND channel_id=$2",
    )
    .bind(f.community.as_uuid())
    .bind(f.channel)
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(
        f.state
            .get_accessible_channel_ids_cached(f.community, &f.keys.public_key().to_bytes())
            .await
            .unwrap()
            .contains(&f.channel),
        "control: cached access must still be stale"
    );
    let (status, revoked) = f.post(&f.keys, "/query", json!([filter])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(revoked, json!([]));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_real_query_validation_forgery_and_aux_failure() {
    let f = Fixture::new().await;
    for (key, val) in [
        ("until", json!(1)),
        ("before_id", json!("z".repeat(64))),
        ("top_level", json!(true)),
        ("thread_cursor", json!(1)),
        ("depth_limit", json!(0)),
        ("authors", json!([f.keys.public_key().to_hex()])),
        ("page", json!(1)),
    ] {
        let mut filter = f.filter();
        filter[key] = val;
        let (status, body) = f.post(&f.keys, "/query", json!([filter])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    let forged = event(
        &f.keys,
        f.channel,
        39007,
        "{}",
        None,
        Timestamp::now().as_secs(),
    );
    let (status, body) = f
        .post(&f.keys, "/events", serde_json::to_value(forged).unwrap())
        .await;
    assert!(
        !status.is_success() || body["accepted"] == false,
        "{status}: {body}"
    );
    assert!(body.to_string().contains("relay-only"), "{body}");
    let aux = f.aux(40003, &f.root, Some(f.channel)).await;
    sqlx::query("UPDATE events SET sig='\\x00' WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(aux.id.to_bytes().to_vec())
        .execute(&f.pool)
        .await
        .unwrap();
    let (status, body) = f.post(&f.keys, "/query", json!([f.filter()])).await;
    assert!(status.is_server_error(), "{status}: {body}");
    assert!(!body.is_array());
    // A required SQL read blocked by DDL must error, not sign empty bounds.
    let mut lock = f.pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE thread_metadata IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let (status, body) = f.post(&f.keys, "/query", json!([f.filter()])).await;
    assert!(status.is_server_error(), "{status}: {body}");
    lock.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_scope_roots_and_aggregate_budgets() {
    let f = Fixture::new().await;
    let filter = f.filter();
    let (status, body) = f
        .post(&f.keys, "/query", json!(vec![filter.clone(); 5]))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    for other in [
        json!({"kinds":[20001]}),
        json!({"kinds":[9],"search":"x"}),
        json!({"kinds":[9],"thread_cursor":-1,"depth_limit":2147483647}),
    ] {
        let (status, body) = f.post(&f.keys, "/query", json!([filter, other])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    // A private/author-gated non-conversation root cannot authorize aux.
    let hidden = event(
        &f.keys,
        f.channel,
        30300,
        "private root",
        None,
        f.root.created_at.as_secs(),
    );
    f.state
        .db
        .insert_event(f.community, &hidden, Some(f.channel))
        .await
        .unwrap();
    f.aux(40003, &hidden, Some(f.channel)).await;
    let mut hidden_filter = filter.clone();
    hidden_filter["#e"] = json!([hidden.id.to_hex()]);
    let (status, body) = f.post(&f.keys, "/query", json!([hidden_filter])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body.as_array().unwrap().len(),
        1,
        "hidden-root aux must not leak"
    );
    // A root from another channel cannot influence rows, probe or root aux.
    let other_channel = Uuid::new_v4();
    f.state
        .db
        .create_channel_with_id(
            f.community,
            other_channel,
            "other",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            &Keys::generate().public_key().to_bytes(),
            None,
        )
        .await
        .unwrap();
    let other_root = event(
        &f.keys,
        other_channel,
        9,
        "other root",
        None,
        f.root.created_at.as_secs(),
    );
    f.state
        .db
        .insert_event(f.community, &other_root, Some(other_channel))
        .await
        .unwrap();
    f.aux(40003, &other_root, Some(f.channel)).await;
    hidden_filter["#e"] = json!([other_root.id.to_hex()]);
    let (status, body) = f.post(&f.keys, "/query", json!([hidden_filter])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Real bounded payloads: one 4.8 MiB page passes, two in the same request
    // exceed 8 MiB. A per-window (instead of per-query) ledger fails this test.
    let aux = f.aux(40003, &f.root, Some(f.channel)).await;
    sqlx::query("INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig,received_at,channel_id) \
        SELECT community_id,decode(md5(n::text)||md5(('payload'||n)::text),'hex'),pubkey,created_at,kind,tags,repeat('x',60000),sig,received_at,channel_id \
        FROM events CROSS JOIN generate_series(1,80) n WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid()).bind(aux.id.to_bytes().to_vec()).execute(&f.pool).await.unwrap();
    let (status, one) = f.post(&f.keys, "/query", json!([filter])).await;
    assert_eq!(status, StatusCode::OK, "{one}");
    let (status, two) = f.post(&f.keys, "/query", json!([filter, filter])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{two}");
    assert!(two.to_string().contains("byte budget"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_router_aux_row_cap_and_corrupt_page() {
    let f = Fixture::new().await;
    let aux = f.aux(40003, &f.root, Some(f.channel)).await;
    sqlx::query("INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig,received_at,channel_id) \
        SELECT community_id,decode(md5(n::text)||md5(('aux'||n)::text),'hex'),pubkey,created_at,kind,tags,'fixture',sig,received_at,channel_id \
        FROM events CROSS JOIN generate_series(1,8200) n WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid()).bind(aux.id.to_bytes().to_vec()).execute(&f.pool).await.unwrap();
    let (status, body) = f.post(&f.keys, "/query", json!([f.filter()])).await;
    assert!(status.is_server_error(), "{status}: {body}");
    assert_eq!(body, json!({"error":"internal server error"}));
    // Keep 1,001 rows and corrupt one guaranteed to lie on the first raw page.
    sqlx::query("DELETE FROM events WHERE community_id=$1 AND kind=40003 AND id NOT IN \
        (SELECT id FROM events WHERE community_id=$1 AND kind=40003 ORDER BY created_at DESC,id ASC LIMIT 1001)")
        .bind(f.community.as_uuid()).execute(&f.pool).await.unwrap();
    // Positive control: the same 1,001 raw rows succeed before corruption.
    let (status, complete) = f.post(&f.keys, "/query", json!([f.filter()])).await;
    assert_eq!(status, StatusCode::OK, "{complete}");
    assert_eq!(complete.as_array().unwrap().len(), 1002);
    sqlx::query("UPDATE events SET sig='\\x00' WHERE community_id=$1 AND id = \
        (SELECT id FROM events WHERE community_id=$1 AND kind=40003 ORDER BY created_at DESC,id ASC OFFSET 500 LIMIT 1)")
        .bind(f.community.as_uuid()).execute(&f.pool).await.unwrap();
    let (status, body) = f.post(&f.keys, "/query", json!([f.filter()])).await;
    assert!(status.is_server_error(), "{status}: {body}");
    assert_eq!(body, json!({"error":"internal server error"}));
}

mod cost_postgres_tests;
mod failure_postgres_tests;
