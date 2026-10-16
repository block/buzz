//! Router regressions with committed relational state and deliberately stale
//! signed discovery events. This models held/failed publication after commit;
//! it does not inject a failure into the publication worker itself.
use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use base64::Engine;
use buzz_core::{
    channel::{ChannelType, ChannelVisibility, MemberRole},
    CommunityId,
};
use nostr::{EventBuilder, Keys, Kind, Tag};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    state: Arc<AppState>,
    host: String,
    community: CommunityId,
    viewer: Keys,
    destination: Uuid,
    source: Uuid,
    pool: sqlx::PgPool,
}

impl Fixture {
    async fn new() -> Self {
        let fixture = crate::api::bridge::postgres_tests::bridge_handler_test_state()
            .await
            .expect("isolated Postgres/Redis fixture");
        let mut state = (*fixture).clone();
        Arc::make_mut(&mut state.config).require_auth_token = true;
        state.nip98_replay = Arc::new(buzz_pubsub::RedisNip98ReplayGuard::new(
            state.redis_pool.clone(),
        ));
        let state = Arc::new(state);
        let host = format!("audience-{}.local", Uuid::new_v4());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .unwrap()
            .id;
        let viewer = Keys::generate();
        let destination = Uuid::new_v4();
        let source = Uuid::new_v4();
        for id in [destination, source] {
            state
                .db
                .create_channel_with_id(
                    community,
                    id,
                    "authority test",
                    ChannelType::Stream,
                    ChannelVisibility::Private,
                    Some("Buzz session (buzz.sessions/v1)"),
                    &viewer.public_key().to_bytes(),
                    None,
                )
                .await
                .unwrap();
        }
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        Self {
            state,
            host,
            community,
            viewer,
            destination,
            source,
            pool,
        }
    }
    fn filter(&self) -> Value {
        json!([{"kinds":[39000,39002],"consistency":"strong","audience_authority":{
            "version":1,"destination":self.destination,"sources":[self.source]}}])
    }
    async fn post(&self, key: Option<&Keys>, host: &str, value: Value) -> (StatusCode, Value) {
        let body = serde_json::to_vec(&value).unwrap();
        let mut req = Request::builder()
            .method("POST")
            .uri("/query")
            .header("host", host);
        if let Some(key) = key {
            let proof = EventBuilder::new(Kind::Custom(27235), "")
                .tags([
                    Tag::parse(["u", &format!("https://{host}/query")]).unwrap(),
                    Tag::parse(["method", "POST"]).unwrap(),
                    Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
                    Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
                ])
                .sign_with_keys(key)
                .unwrap();
            req = req.header(
                "authorization",
                format!(
                    "Nostr {}",
                    base64::engine::general_purpose::STANDARD
                        .encode(serde_json::to_vec(&proof).unwrap())
                ),
            );
        }
        let response = crate::router::build_router(self.state.clone())
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    async fn allowed(&self, expected: bool) {
        let (status, body) = self
            .post(Some(&self.viewer), &self.host, self.filter())
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["audience_authority"], 1);
        assert_eq!(body["complete"], true);
        assert_eq!(body["community_id"], self.community.as_uuid().to_string());
        assert_eq!(body["pubkey"], self.viewer.public_key().to_hex());
        assert_eq!(body["destination"], self.destination.to_string());
        assert_eq!(
            body["sources"],
            json!([{"id":self.source,"allowed":expected}])
        );
    }
    async fn member(&self, channel: Uuid, key: &Keys) {
        self.state
            .db
            .add_member(
                self.community,
                channel,
                &key.public_key().to_bytes(),
                MemberRole::Member,
                Some(&self.viewer.public_key().to_bytes()),
            )
            .await
            .unwrap();
    }
    async fn visibility(&self, channel: Uuid, visibility: &str) {
        sqlx::query(
            "UPDATE channels SET visibility=$3::channel_visibility WHERE community_id=$1 AND id=$2",
        )
        .bind(self.community.as_uuid())
        .bind(channel)
        .bind(visibility)
        .execute(&self.pool)
        .await
        .unwrap();
    }
    async fn stale_discovery(&self) -> Vec<nostr::Event> {
        let mut events = Vec::new();
        for id in [self.destination, self.source] {
            for kind in [39000, 39002] {
                let mut tags = vec![Tag::identifier(id.to_string())];
                if kind == 39000 {
                    tags.extend([
                        Tag::parse(["name", "authority test"]).unwrap(),
                        Tag::parse(["description", "Buzz session (buzz.sessions/v1)"]).unwrap(),
                        Tag::parse(["type", "stream"]).unwrap(),
                        Tag::parse(["private"]).unwrap(),
                    ]);
                } else {
                    tags.push(
                        Tag::parse(["p", &self.viewer.public_key().to_hex(), "", "owner"]).unwrap(),
                    );
                }
                let event = EventBuilder::new(Kind::Custom(kind), "")
                    .tags(tags)
                    .sign_with_keys(&self.state.relay_keypair)
                    .unwrap();
                assert_eq!(event.pubkey, self.state.relay_keypair.public_key());
                event.verify().unwrap();
                self.state
                    .db
                    .replace_parameterized_event(self.community, &event, &id.to_string(), Some(id))
                    .await
                    .unwrap();
                events.push(event);
            }
        }
        events
    }
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn committed_membership_overrides_stale_discovery() {
    let f = Fixture::new().await;
    let events = f.stale_discovery().await;
    f.allowed(true).await;
    let newcomer = Keys::generate();
    f.member(f.destination, &newcomer).await;
    f.allowed(false).await;
    f.member(f.source, &newcomer).await;
    f.allowed(true).await;
    f.state
        .db
        .remove_member(
            f.community,
            f.source,
            &newcomer.public_key().to_bytes(),
            &f.viewer.public_key().to_bytes(),
        )
        .await
        .unwrap();
    f.allowed(false).await;
    // None of these committed changes published replacements. Signed event
    // bodies remain byte-for-byte the original incomplete discovery evidence.
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE community_id=$1 AND kind IN (39000,39002)",
    )
    .bind(f.community.as_uuid())
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 4, "no replacement discovery events were published");
    for event in events {
        let (content, tags, sig): (String, Value, Vec<u8>) =
            sqlx::query_as("SELECT content,tags,sig FROM events WHERE community_id=$1 AND id=$2")
                .bind(f.community.as_uuid())
                .bind(event.id.to_bytes().to_vec())
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(content, event.content);
        assert_eq!(tags, serde_json::to_value(&event.tags).unwrap());
        assert_eq!(sig, event.sig.as_ref().to_vec());
    }
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn current_visibility_session_identity_and_viewer_access() {
    let f = Fixture::new().await;
    f.stale_discovery().await;
    f.allowed(true).await;
    f.visibility(f.destination, "open").await;
    f.allowed(false).await;
    f.visibility(f.destination, "private").await;
    f.allowed(true).await;
    f.visibility(f.source, "open").await;
    f.allowed(false).await;
    f.visibility(f.source, "private").await;
    f.allowed(true).await;
    sqlx::query(
        "UPDATE channels SET description='ordinary channel' WHERE community_id=$1 AND id=$2",
    )
    .bind(f.community.as_uuid())
    .bind(f.source)
    .execute(&f.pool)
    .await
    .unwrap();
    f.allowed(false).await;
    sqlx::query(
        "UPDATE channels SET description='Buzz session (buzz.sessions/v1)' WHERE community_id=$1 AND id=$2",
    )
    .bind(f.community.as_uuid())
    .bind(f.source)
    .execute(&f.pool)
    .await
    .unwrap();
    f.allowed(true).await;
    let outsider = Keys::generate();
    let (status, body) = f.post(Some(&outsider), &f.host, f.filter()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sources"], json!([{"id":f.source,"allowed":false}]));
    assert_eq!(
        f.post(None, &f.host, f.filter()).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn strict_contract_and_cross_community_isolation() {
    let f = Fixture::new().await;
    let other_host = format!("other-audience-{}.local", Uuid::new_v4());
    f.state
        .db
        .ensure_configured_community(&other_host)
        .await
        .unwrap();
    let (status, body) = f.post(Some(&f.viewer), &other_host, f.filter()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sources"], json!([{"id":f.source,"allowed":false}]));
    let base = f.filter();
    let mut bad = Vec::new();
    for version in [json!(2), json!("1"), Value::Null] {
        let mut v = base.clone();
        v[0]["audience_authority"]["version"] = version;
        bad.push(v);
    }
    for sources in [
        json!([]),
        json!([f.source, f.source]),
        json!(["invalid"]),
        json!([f.source.to_string().to_uppercase()]),
        json!((0..101).map(|_| Uuid::new_v4()).collect::<Vec<_>>()),
    ] {
        let mut v = base.clone();
        v[0]["audience_authority"]["sources"] = sources;
        bad.push(v);
    }
    let mut extra = base.clone();
    extra[0]["limit"] = json!(1);
    bad.push(extra);
    let mut mixed = base.clone();
    mixed.as_array_mut().unwrap().push(json!({"kinds":[9]}));
    bad.push(mixed);
    for value in bad {
        let (status, body) = f.post(Some(&f.viewer), &f.host, value.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "request {value}: {body}");
        assert_ne!(body["complete"], true);
    }
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn audience_coverage_does_not_truncate_large_rosters() {
    let f = Fixture::new().await;
    // Deterministic 32-byte keys are adequate relational membership identities.
    sqlx::query("INSERT INTO channel_members (community_id,channel_id,pubkey) SELECT $1,$2,decode(md5(n::text)||md5(('audience'||n)::text),'hex') FROM generate_series(1,1100) n")
        .bind(f.community.as_uuid()).bind(f.destination).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO channel_members (community_id,channel_id,pubkey) SELECT community_id,$3,pubkey FROM channel_members WHERE community_id=$1 AND channel_id=$2 ON CONFLICT DO NOTHING")
        .bind(f.community.as_uuid()).bind(f.destination).bind(f.source).execute(&f.pool).await.unwrap();
    f.allowed(true).await;
    let last = Keys::generate();
    f.member(f.destination, &last).await;
    f.allowed(false).await;
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn nip11_advertises_only_resolved_community_authority() {
    let f = Fixture::new().await;
    let info = crate::nip11::nip11_document(&f.state, &f.host).await;
    assert_eq!(
        info.audience_authority,
        Some(json!({
            "version":1,"community_id":f.community.as_uuid(),"max_sources":100
        }))
    );
    let unknown = format!("unknown-audience-{}.invalid", Uuid::new_v4());
    let info = crate::nip11::nip11_document(&f.state, &unknown).await;
    assert!(info.audience_authority.is_none());
}

/// Exercise the deployed HTTP boundary over an actual loopback listener, not
/// only Router::oneshot. All records belong to a disposable fixture community.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn local_listener_checks_committed_audience_with_stale_discovery() {
    let f = Fixture::new().await;
    f.stale_discovery().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let router = crate::router::build_router(f.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stop_rx.await;
            })
            .await
            .unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap();
    let body = serde_json::to_vec(&f.filter()).unwrap();
    let query = || {
        let proof = EventBuilder::new(Kind::Custom(27235), "")
            .tags([
                Tag::parse(["u", &format!("https://{}/query", f.host)]).unwrap(),
                Tag::parse(["method", "POST"]).unwrap(),
                Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
                Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
            ])
            .sign_with_keys(&f.viewer)
            .unwrap();
        client
            .post(format!("http://{addr}/query"))
            .header("host", &f.host)
            .header(
                "authorization",
                format!(
                    "Nostr {}",
                    base64::engine::general_purpose::STANDARD
                        .encode(serde_json::to_vec(&proof).unwrap())
                ),
            )
            .body(body.clone())
            .send()
    };
    let before = query().await.unwrap();
    let before_status = before.status();
    let before: Value = before.json().await.unwrap();
    f.member(f.destination, &Keys::generate()).await;
    let after = query().await.unwrap();
    let after_status = after.status();
    let after: Value = after.json().await.unwrap();
    stop_tx.send(()).unwrap();
    server.await.unwrap();
    assert_eq!(before_status, StatusCode::OK, "{before}");
    assert_eq!(after_status, StatusCode::OK, "{after}");
    assert_eq!(before["sources"], json!([{"id":f.source,"allowed":true}]));
    assert_eq!(after["sources"], json!([{"id":f.source,"allowed":false}]));
    assert_eq!(after["complete"], true);
    assert_eq!(after["community_id"], f.community.as_uuid().to_string());
}
