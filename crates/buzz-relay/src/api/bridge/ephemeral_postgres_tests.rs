//! Exercise the HTTP router, shared ephemeral path and delivery gates together.

use std::{collections::HashMap, sync::atomic::AtomicU8, sync::Arc};

use axum::{body::Body, extract::ws::Message, http::Request};
use base64::Engine;
use buzz_core::{kind::KIND_AGENT_ACTIVITY_SNAPSHOT, CommunityId};
use nostr::{Event, EventBuilder, Filter, Keys, Kind, Tag};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use uuid::Uuid;

use crate::state::AppState;

fn watch(
    state: &AppState,
    community: CommunityId,
    channel: Uuid,
    keys: &Keys,
) -> mpsc::Receiver<Message> {
    let conn = Uuid::new_v4();
    let (tx, rx) = mpsc::channel(8);
    let (ctrl, _ctrl_rx) = mpsc::channel(8);
    state.conn_manager.register(
        conn,
        tx,
        ctrl,
        None,
        CancellationToken::new(),
        community,
        Arc::new(AtomicU8::new(0)),
        Arc::new(Mutex::new(HashMap::new())),
        3,
    );
    state
        .conn_manager
        .set_authenticated_pubkey(conn, keys.public_key().to_bytes().to_vec());
    state.sub_registry.register_scoped(
        community,
        conn,
        "activity".into(),
        vec![Filter::new().kind(Kind::Custom(KIND_AGENT_ACTIVITY_SNAPSHOT as u16))],
        Some(channel),
    );
    rx
}

fn event(keys: &Keys, channel: Uuid, seq: u64) -> Event {
    EventBuilder::new(
        Kind::Custom(KIND_AGENT_ACTIVITY_SNAPSHOT as u16),
        format!("{{\"seq\":{seq}}}"),
    )
    .tags([Tag::parse(["h", &channel.to_string()]).unwrap()])
    .sign_with_keys(keys)
    .unwrap()
}

async fn post(
    state: &Arc<AppState>,
    host: &str,
    keys: &Keys,
    event: &Event,
) -> (u16, serde_json::Value) {
    let body = serde_json::to_vec(event).unwrap();
    let auth = EventBuilder::new(Kind::HttpAuth, "")
        .tags([
            Tag::parse(["u", &format!("https://{host}/events")]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&auth).unwrap());
    let response = crate::router::build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/events")
                .header("host", host)
                .header("authorization", format!("Nostr {encoded}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn received_id(rx: &mut mpsc::Receiver<Message>) -> String {
    let Message::Text(text) = rx.try_recv().expect("authorized subscriber receives event") else {
        panic!("expected event text");
    };
    let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(frame[0], "EVENT");
    frame[2]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis"]
async fn http_ephemeral_publish_failure_is_not_acknowledged_or_fanned_out() {
    let mut state = super::postgres_tests::bridge_handler_test_state()
        .await
        .expect("test infrastructure");
    // Keep HTTP admission on healthy Redis, but fail the actual publication.
    let dead_pool = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .unwrap();
    Arc::get_mut(&mut state).unwrap().pubsub = Arc::new(
        buzz_pubsub::PubSubManager::new("redis://127.0.0.1:1", dead_pool)
            .await
            .unwrap(),
    );
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let community = Uuid::new_v4();
    let host = format!("ephemeral-outage-{community}.example");
    sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
        .bind(community)
        .bind(&host)
        .execute(&pool)
        .await
        .unwrap();
    let channel = Uuid::new_v4();
    let publisher = Keys::generate();
    sqlx::query("INSERT INTO channels (community_id,id,name,visibility,created_by) VALUES ($1,$2,'activity','open',$3)")
        .bind(community).bind(channel).bind(publisher.public_key().to_bytes().to_vec())
        .execute(&pool).await.unwrap();
    let community_id = CommunityId::from_uuid(community);
    let mut observer = watch(&state, community_id, channel, &publisher);
    let snapshot = event(&publisher, channel, 1);
    assert_eq!(post(&state, &host, &publisher, &snapshot).await.0, 500);
    assert!(
        observer.try_recv().is_err(),
        "failed publication cannot fan out locally"
    );
    assert!(state
        .local_event_ids
        .get(&(community_id, snapshot.id.to_bytes()))
        .is_none());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1")
        .bind(community)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    sqlx::query("DELETE FROM channels WHERE community_id=$1")
        .bind(community)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM communities WHERE id=$1")
        .bind(community)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis"]
async fn signed_http_ephemeral_is_private_tenant_scoped_and_never_persisted() {
    let state = super::postgres_tests::bridge_handler_test_state()
        .await
        .expect("test infrastructure");
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let host_a = format!("ephemeral-{a}.example");
    let host_b = format!("ephemeral-{b}.example");
    for (id, host) in [(a, &host_a), (b, &host_b)] {
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(id)
            .bind(host)
            .execute(&pool)
            .await
            .unwrap();
    }
    let channel = Uuid::new_v4();
    let publisher = Keys::generate();
    let member = Keys::generate();
    let outsider = Keys::generate();
    for community in [a, b] {
        sqlx::query("INSERT INTO channels (community_id,id,name,visibility,created_by) VALUES ($1,$2,'activity','private',$3)")
            .bind(community).bind(channel).bind(publisher.public_key().to_bytes().to_vec())
            .execute(&pool).await.unwrap();
    }
    for keys in [&publisher, &member] {
        sqlx::query(
            "INSERT INTO channel_members (community_id,channel_id,pubkey) VALUES ($1,$2,$3)",
        )
        .bind(a)
        .bind(channel)
        .bind(keys.public_key().to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
    }
    let community = CommunityId::from_uuid(a);
    let mut authorized = watch(&state, community, channel, &member);
    let mut denied = watch(&state, community, channel, &outsider);
    let mut other_community = watch(&state, CommunityId::from_uuid(b), channel, &member);
    let first = event(&publisher, channel, 1);
    let result = post(&state, &host_a, &publisher, &first).await;
    assert_eq!(result.0, 200, "{result:?}");
    assert_eq!(result.1["accepted"], true);
    assert_eq!(received_id(&mut authorized), first.id.to_hex());
    assert!(denied.try_recv().is_err());
    assert!(other_community.try_recv().is_err());

    // Revocation invalidates the real membership cache. A stale subscription
    // must not be sufficient to receive a subsequent event.
    sqlx::query("UPDATE channel_members SET removed_at=now() WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3")
        .bind(a).bind(channel).bind(member.public_key().to_bytes().to_vec())
        .execute(&pool).await.unwrap();
    let tenant = buzz_core::tenant::TenantContext::resolved(community, host_a.clone());
    state.invalidate_membership(&tenant, channel, &member.public_key().to_bytes());
    assert_eq!(
        post(&state, &host_a, &publisher, &event(&publisher, channel, 2))
            .await
            .0,
        200
    );
    assert!(authorized.try_recv().is_err());
    assert!(denied.try_recv().is_err());
    assert_eq!(
        post(&state, &host_a, &member, &event(&member, channel, 3))
            .await
            .0,
        400,
        "revoked members cannot publish into the private channel"
    );

    // Nonmembers cannot publish into private channels, including an identically
    // numbered channel in another community. Auth signer mismatch also fails.
    assert_eq!(
        post(&state, &host_a, &outsider, &event(&outsider, channel, 3))
            .await
            .0,
        400
    );
    assert_eq!(
        post(&state, &host_b, &publisher, &event(&publisher, channel, 4))
            .await
            .0,
        400
    );
    assert_eq!(
        post(&state, &host_a, &outsider, &event(&publisher, channel, 5))
            .await
            .0,
        403
    );
    let mut tampered = serde_json::to_value(event(&publisher, channel, 6)).unwrap();
    tampered["sig"] = serde_json::Value::String("0".repeat(128));
    let tampered: Event = serde_json::from_value(tampered).unwrap();
    assert_eq!(post(&state, &host_a, &publisher, &tampered).await.0, 400);

    // Ephemeral HTTP submissions must never enter persistence (which is also
    // the source for history, unread counters and message push side effects).
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id IN ($1,$2)")
            .bind(a)
            .bind(b)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    for kind in [
        buzz_core::kind::KIND_AUTH,
        buzz_core::kind::KIND_AGENT_OBSERVER_FRAME,
        buzz_core::kind::KIND_PRESENCE_UPDATE,
    ] {
        let special = EventBuilder::new(Kind::Custom(kind as u16), "")
            .sign_with_keys(&publisher)
            .unwrap();
        assert_eq!(post(&state, &host_a, &publisher, &special).await.0, 400);
    }

    // Open visibility preserves the existing nonmember read/write policy.
    sqlx::query("UPDATE channels SET visibility='open' WHERE community_id=$1 AND id=$2")
        .bind(a)
        .bind(channel)
        .execute(&pool)
        .await
        .unwrap();
    state
        .channel_visibility_cache
        .invalidate(&(community, channel));
    let open = event(&outsider, channel, 7);
    assert_eq!(post(&state, &host_a, &outsider, &open).await.0, 200);
    assert_eq!(received_id(&mut authorized), open.id.to_hex());
    assert_eq!(received_id(&mut denied), open.id.to_hex());
    assert!(other_community.try_recv().is_err());

    use buzz_db::deletion::{
        FrozenInventory, KeyStreamDigest, PrefixManifest, StorageManifest, DEFAULT_LEASE_DURATION,
    };
    let store = buzz_deletion::store(&state.db);
    let request = store.submit(&host_a, "test-operator", None).await.unwrap();
    let inventory = FrozenInventory {
        schema: store.inventory_schema(community).await.unwrap(),
        storage: StorageManifest {
            version: 4,
            prefixes: buzz_media::tenant_prefixes(a)
                .into_iter()
                .map(|prefix| PrefixManifest {
                    prefix,
                    object_count: 0,
                    total_bytes: 0,
                    keys_digest: KeyStreamDigest::new().finish().0,
                })
                .collect(),
        },
    };
    store
        .freeze_inventory(request.id, &inventory)
        .await
        .unwrap();
    store
        .approve(request.id, "test-approver", None)
        .await
        .unwrap();
    let claim = store
        .claim_specific(request.id, "test-executor", DEFAULT_LEASE_DURATION)
        .await
        .unwrap()
        .unwrap();
    store.begin_quiescing(&claim.lease).await.unwrap();
    store.fence(&claim.lease).await.unwrap();
    assert_ne!(
        post(&state, &host_a, &publisher, &event(&publisher, channel, 8))
            .await
            .0,
        200
    );
    assert!(authorized.try_recv().is_err());
}
