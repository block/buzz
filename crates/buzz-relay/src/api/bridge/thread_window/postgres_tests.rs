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
        state.db = production_db(buzz_db::DbConfig::default()).await;
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
        private_channel(&state.db, community, channel, &keys).await;
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
    fn ws_conn(
        &self,
        allowed: Option<Vec<Uuid>>,
    ) -> (
        Arc<crate::connection::ConnectionState>,
        tokio::sync::mpsc::Receiver<axum::extract::ws::Message>,
    ) {
        let auth = buzz_auth::AuthContext {
            pubkey: self.keys.public_key(),
            scopes: vec![],
            channel_ids: allowed,
            auth_method: buzz_auth::AuthMethod::Nip42,
            agent_owner_pubkey: None,
        };
        let (mut conn, rx) = crate::connection::tests::test_conn_with_auth(
            crate::connection::AuthState::Authenticated(auth),
        );
        Arc::get_mut(&mut conn).unwrap().tenant =
            TenantContext::resolved(self.community, self.host.clone());
        (conn, rx)
    }
    async fn post(&self, key: &Keys, path: &str, body: Value) -> (StatusCode, Value) {
        post(self.state.clone(), &self.host, key, path, body).await
    }
    async fn query(&self, filter: &Value) -> Value {
        let (status, body) = self.post(&self.keys, "/query", json!([filter])).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }
    async fn tombstone(&self, event: &Event) {
        sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
            .bind(self.community.as_uuid())
            .bind(event.id.to_bytes().to_vec())
            .execute(&self.pool)
            .await
            .unwrap();
    }
    // Bulk budget fixtures need structurally readable rows, not valid signatures.
    async fn copy_aux(&self, source: &Event, count: i32, content: &str) {
        sqlx::query("INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig,received_at,channel_id) \
            SELECT community_id,decode(md5(n::text)||md5(('aux'||n)::text),'hex'),pubkey,created_at,kind,tags,$4,sig,received_at,channel_id \
            FROM events CROSS JOIN generate_series(1,$3) n WHERE community_id=$1 AND id=$2")
            .bind(self.community.as_uuid()).bind(source.id.to_bytes().to_vec())
            .bind(count).bind(content).execute(&self.pool).await.unwrap();
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
            channel.unwrap_or(self.channel),
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
        self.bounds_on_host(response, filter, &self.host)
    }
    fn bounds_on_host(&self, response: &Value, filter: &Value, host: &str) -> Value {
        let request = Request::parse(filter).unwrap();
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
                ["d", request.binding(host, &self.keys.public_key().to_hex())],
                ["h", request.channel.to_string()],
                ["e", request.root]
            ])
        );
        let content: Value = serde_json::from_str(&event.content).unwrap();
        assert_eq!(content["version"], 1);
        assert_eq!(content["direction"], "older");
        assert_eq!(
            content["has_more"].as_bool().unwrap(),
            !content["next_cursor"].is_null()
        );
        content
    }
}

async fn production_db(mut config: buzz_db::DbConfig) -> buzz_db::Db {
    config.database_url = crate::test_support::database_url();
    config.max_connections = 5;
    buzz_db::Db::new(&config).await.unwrap()
}

async fn private_channel(db: &buzz_db::Db, community: CommunityId, channel: Uuid, owner: &Keys) {
    db.create_channel_with_id(
        community,
        channel,
        "thread-window",
        ChannelType::Stream,
        ChannelVisibility::Private,
        None,
        &owner.public_key().to_bytes(),
        None,
    )
    .await
    .unwrap();
}

fn ids(response: &Value, kind: Option<u16>) -> Vec<&str> {
    response
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| kind.is_none_or(|kind| e["kind"] == kind))
        .map(|e| e["id"].as_str().unwrap())
        .collect()
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
    let empty = f.query(&filter).await;
    assert_eq!(f.bounds(&empty, &filter)["has_more"], false);
    let mut replies = Vec::new();
    for n in 0..51 {
        replies.push(f.reply(n).await);
    }
    let sentinel_aux = f.aux(7, &replies[0], Some(f.channel)).await;
    let reaction = f.aux(7, &replies[50], Some(f.channel)).await;
    let deletion = f.aux(5, &reaction, None).await;
    f.tombstone(&reaction).await;
    let root_edit = f.aux(40003, &f.root, Some(f.channel)).await;
    let page = f.query(&filter).await;
    let rows = ids(&page, Some(9));
    assert_eq!(rows.len(), 50);
    assert_eq!(rows[0], replies[50].id.to_hex());
    assert_eq!(rows[49], replies[1].id.to_hex());
    let all = ids(&page, None);
    for (event, present) in [
        (&sentinel_aux, false),
        (&reaction, false),
        (&deletion, true),
        (&root_edit, true),
    ] {
        assert_eq!(all.contains(&event.id.to_hex().as_str()), present);
    }
    let bounds = f.bounds(&page, &filter);
    assert_eq!(bounds["has_more"], true);
    assert_eq!(bounds["next_cursor"]["id"], replies[1].id.to_hex());
    let mut next = filter.clone();
    next["until"] = bounds["next_cursor"]["created_at"].clone();
    next["before_id"] = bounds["next_cursor"]["id"].clone();
    let tail = f.query(&next).await;
    assert_eq!(f.bounds(&tail, &next)["next_cursor"], Value::Null);
    assert_eq!(ids(&tail, Some(9)), [replies[0].id.to_hex()]);
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
            let old = f.query(&legacy).await;
            assert_eq!(old[0]["id"], replies[0].id.to_hex());
            assert_eq!(old[1]["id"], replies[1].id.to_hex());
            assert_eq!(old.as_array().unwrap().len(), 2);
            legacy[ts_key] = json!(replies[1].created_at.as_secs());
            legacy[id_key] = json!(replies[1].id.to_hex());
            let next = f.query(&legacy).await;
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
    private_channel(&f.state.db, other, f.channel, &f.keys).await;
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
    assert_eq!(
        other_page,
        json!([]),
        "absent root must not sign exhaustion"
    );
    f.query(&filter).await;
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
async fn websocket_batch_enforces_token_scope_and_rejects_queue_truncation() {
    let f = Fixture::new().await;
    f.reply(1).await;
    let filter = f.filter();
    let parsed = nostr::Filter::new();

    let (denied, mut denied_rx) = f.ws_conn(Some(vec![]));
    crate::handlers::req::handle_req(
        "denied".into(),
        vec![parsed.clone()],
        vec![filter.clone()],
        vec![None],
        denied,
        f.state.clone(),
    )
    .await;
    assert_eq!(frame(&mut denied_rx)[0], "EOSE");
    assert!(
        denied_rx.try_recv().is_err(),
        "token-excluded channel emitted rows"
    );

    let (allowed, mut allowed_rx) = f.ws_conn(Some(vec![f.channel]));
    crate::handlers::req::handle_req(
        "allowed".into(),
        vec![parsed.clone()],
        vec![filter.clone()],
        vec![None],
        allowed.clone(),
        f.state.clone(),
    )
    .await;
    let mut frames = vec![frame(&mut allowed_rx), frame(&mut allowed_rx)];
    frames.push(frame(&mut allowed_rx));
    assert_eq!(
        frames.iter().map(|v| v[0].as_str()).collect::<Vec<_>>(),
        vec![Some("EVENT"), Some("EVENT"), Some("EOSE")]
    );
    assert_eq!(frames[1][2]["kind"], 39007);
    assert!(
        !allowed.cancel.is_cancelled(),
        "complete page closed socket"
    );

    // A slow but draining writer must receive the entire page even when its
    // queue holds fewer frames than the window produces.
    let (draining, mut draining_rx) = f.ws_conn(None);
    let drain = tokio::spawn(async move {
        let mut frames = Vec::new();
        for _ in 0..7 {
            let message = draining_rx.recv().await.expect("complete window frame");
            frames.push(serde_json::from_str::<Value>(message.to_text().unwrap()).unwrap());
        }
        frames
    });
    crate::handlers::req::handle_req(
        "draining".into(),
        vec![parsed.clone(); 3],
        vec![filter.clone(); 3],
        vec![None; 3],
        draining.clone(),
        f.state.clone(),
    )
    .await;
    let drained = tokio::time::timeout(std::time::Duration::from_secs(2), drain)
        .await
        .expect("draining writer completes")
        .unwrap();
    assert_eq!(drained.len(), 7);
    assert!(drained[..6].iter().all(|frame| frame[0] == "EVENT"));
    assert_eq!(drained[6][0], "EOSE");
    assert!(!draining.cancel.is_cancelled());

    // A queue with room for the page but not EOSE must close, not complete.
    let (terminal, mut terminal_rx) = f.ws_conn(None);
    for _ in 0..1 {
        assert!(terminal.send("padding".into()));
    }
    crate::handlers::req::handle_req(
        "terminal".into(),
        vec![parsed.clone()],
        vec![filter.clone()],
        vec![None],
        terminal.clone(),
        f.state.clone(),
    )
    .await;
    assert!(
        terminal.cancel.is_cancelled(),
        "failed EOSE must close socket"
    );
    let terminal_frames: Vec<_> = std::iter::from_fn(|| terminal_rx.try_recv().ok())
        .filter_map(|msg| serde_json::from_str::<Value>(msg.to_text().unwrap()).ok())
        .collect();
    assert_eq!(terminal_frames.len(), 2);
    assert!(terminal_frames.iter().all(|frame| frame[0] == "EVENT"));

    // The source reports truncation if delivery cannot proceed by its bound.
    let (probe, _probe_rx) = f.ws_conn(None);
    let requests = super::parse(&vec![filter.clone(); 3]).unwrap();
    let sent = crate::handlers::req::serve_thread_windows(
        "full",
        requests.iter().flatten(),
        None,
        &f.keys.public_key(),
        &probe,
        &f.state,
        tokio::time::Instant::now() + std::time::Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert!(!sent, "a failed EVENT enqueue must not authorize EOSE");

    // A stalled writer cannot receive EOSE; the finite request must close.
    let (slow, mut slow_rx) = f.ws_conn(None);
    crate::handlers::req::handle_req(
        "full".into(),
        vec![parsed.clone(); 3],
        vec![filter.clone(); 3],
        vec![None; 3],
        slow.clone(),
        f.state.clone(),
    )
    .await;
    assert!(
        slow.cancel.is_cancelled(),
        "truncated page must close socket"
    );
    let drained: Vec<_> = std::iter::from_fn(|| slow_rx.try_recv().ok())
        .map(|m| serde_json::from_str::<Value>(m.to_text().unwrap()).unwrap())
        .collect();
    assert_eq!(drained.len(), 3);
    assert!(
        drained.iter().all(|frame| frame[0] == "EVENT"),
        "{drained:?}"
    );

    let (invalid, mut invalid_rx) = f.ws_conn(None);
    let mut bad = filter.clone();
    bad["before_id"] = json!("zz");
    bad["until"] = json!(1);
    crate::handlers::req::handle_req(
        "bad".into(),
        vec![parsed.clone()],
        vec![bad],
        vec![None],
        invalid,
        f.state.clone(),
    )
    .await;
    assert_eq!(frame(&mut invalid_rx)[0], "CLOSED");
    assert!(invalid_rx.try_recv().is_err());

    let (mixed, mut mixed_rx) = f.ws_conn(None);
    crate::handlers::req::handle_req(
        "mixed".into(),
        vec![parsed; 2],
        vec![filter, json!({"kinds":[9]})],
        vec![None; 2],
        mixed,
        f.state.clone(),
    )
    .await;
    assert_eq!(frame(&mut mixed_rx)[0], "CLOSED");
    assert!(mixed_rx.try_recv().is_err());

    // A rejected window with a saturated terminal queue must not leave its
    // subscription waiting on a socket that will never deliver CLOSED.
    let (rejected, _rejected_rx) = f.ws_conn(None);
    for _ in 0..4 {
        assert!(rejected.send("padding".into()));
    }
    crate::handlers::req::handle_req(
        "invalid-full".into(),
        vec![nostr::Filter::new()],
        vec![json!({"thread_window":true,"until":1})],
        vec![None],
        rejected.clone(),
        f.state.clone(),
    )
    .await;
    assert!(rejected.cancel.is_cancelled());
}

fn frame(rx: &mut tokio::sync::mpsc::Receiver<axum::extract::ws::Message>) -> Value {
    serde_json::from_str(rx.try_recv().expect("queued frame").to_text().unwrap()).unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn websocket_window_replaces_live_subscription_before_eose() {
    let f = Fixture::new().await;
    f.reply(1).await;
    let (conn, mut rx) = f.ws_conn(None);
    let live_filter = nostr::Filter::new().kind(Kind::Custom(9));
    conn.subscriptions
        .lock()
        .await
        .insert("replace".into(), vec![live_filter.clone()]);
    f.state.sub_registry.register_scoped(
        f.community,
        conn.conn_id,
        "replace".into(),
        vec![live_filter],
        Some(f.channel),
    );
    f.state
        .pubsub
        .retain_topic(&conn.tenant, buzz_pubsub::EventTopic::Channel(f.channel))
        .await;
    assert_eq!(
        f.state
            .pubsub
            .topic_refcount(&conn.tenant, buzz_pubsub::EventTopic::Channel(f.channel))
            .await,
        1,
        "fixture must retain the live channel topic"
    );
    assert_eq!(
        f.state
            .sub_registry
            .channel_subscriber_conns_scoped(f.community, f.channel),
        vec![conn.conn_id]
    );

    crate::handlers::req::handle_req(
        "replace".into(),
        vec![nostr::Filter::new()],
        vec![f.filter()],
        vec![None],
        conn.clone(),
        f.state.clone(),
    )
    .await;
    assert!(conn.subscriptions.lock().await.get("replace").is_none());
    assert_eq!(
        f.state
            .pubsub
            .topic_refcount(&conn.tenant, buzz_pubsub::EventTopic::Channel(f.channel))
            .await,
        0,
        "replacing the live subscription must release its channel topic"
    );
    assert!(f
        .state
        .sub_registry
        .channel_subscriber_conns_scoped(f.community, f.channel)
        .is_empty());
    assert_eq!(frame(&mut rx)[0], "EVENT");
    assert_eq!(frame(&mut rx)[0], "EVENT");
    assert_eq!(frame(&mut rx)[0], "EOSE");
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_real_query_validation_forgery_and_sql_failure() {
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
    // Neither a private non-conversation root nor a root in another channel
    // may authorize aux, even when the aux itself is in the requested channel.
    let other_channel = Uuid::new_v4();
    private_channel(&f.state.db, f.community, other_channel, &Keys::generate()).await;
    for (channel, kind) in [(f.channel, 30300), (other_channel, 9)] {
        let root = event(
            &f.keys,
            channel,
            kind,
            "hidden root",
            None,
            f.root.created_at.as_secs(),
        );
        f.state
            .db
            .insert_event(f.community, &root, Some(channel))
            .await
            .unwrap();
        f.aux(40003, &root, Some(f.channel)).await;
        let mut hidden_filter = filter.clone();
        hidden_filter["#e"] = json!([root.id.to_hex()]);
        let body = f.query(&hidden_filter).await;
        assert_eq!(
            body,
            json!([]),
            "hidden roots return neither aux nor bounds"
        );
    }

    // Real bounded payloads: one 4.8 MiB page passes, two in the same request
    // exceed 8 MiB. A per-window (instead of per-query) ledger fails this test.
    let aux = f.aux(40003, &f.root, Some(f.channel)).await;
    f.copy_aux(&aux, 80, &"x".repeat(60_000)).await;
    f.query(&filter).await;
    let (status, two) = f.post(&f.keys, "/query", json!([filter, filter])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{two}");
    assert!(two.to_string().contains("byte budget"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_router_aux_row_cap_and_corrupt_page() {
    let f = Fixture::new().await;
    let aux = f.aux(40003, &f.root, Some(f.channel)).await;
    for n in 0..51 {
        f.reply(n).await;
    }
    f.copy_aux(&aux, 8200, "fixture").await;
    let (status, body) = f.post(&f.keys, "/query", json!([f.filter()])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body["error"].as_str().unwrap().contains("raw row budget"));
    // Keep 1,001 rows and corrupt one guaranteed to lie on the first raw page.
    sqlx::query("DELETE FROM events WHERE community_id=$1 AND kind=40003 AND id NOT IN \
        (SELECT id FROM events WHERE community_id=$1 AND kind=40003 ORDER BY created_at DESC,id ASC LIMIT 1001)")
        .bind(f.community.as_uuid()).execute(&f.pool).await.unwrap();
    // Positive control: the same 1,001 raw rows succeed before corruption.
    let complete = f.query(&f.filter()).await;
    assert_eq!(complete.as_array().unwrap().len(), 1052);
    assert_eq!(f.bounds(&complete, &f.filter())["has_more"], true);
    sqlx::query("UPDATE events SET sig='\\x00' WHERE community_id=$1 AND id = \
        (SELECT id FROM events WHERE community_id=$1 AND kind=40003 ORDER BY created_at DESC,id ASC OFFSET 500 LIMIT 1)")
        .bind(f.community.as_uuid()).execute(&f.pool).await.unwrap();
    let (status, body) = f.post(&f.keys, "/query", json!([f.filter()])).await;
    assert!(status.is_server_error(), "{status}: {body}");
    assert_eq!(body, json!({"error":"internal server error"}));
}

mod failure_postgres_tests;

mod review_postgres_tests;
