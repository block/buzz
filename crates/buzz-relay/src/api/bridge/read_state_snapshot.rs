//! Opt-in atomic full-state query extension. Ordinary query arrays cannot
//! accidentally satisfy this envelope, even on relays that ignore extensions.

use axum::{http::StatusCode, Json};
use buzz_core::TenantContext;
use serde_json::{json, Value};

use crate::{api::api_error, state::AppState};

pub(super) fn requested(filters: &[Value]) -> bool {
    filters
        .iter()
        .any(|f| f.get("read_state_snapshot").is_some())
}

pub(super) async fn query(
    state: &AppState,
    tenant: &TenantContext,
    author: &nostr::PublicKey,
    filters: &[Value],
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Exact raw shape: never silently drop constraints or unknown versions.
    let expected = json!({
        "kinds": [buzz_core::kind::KIND_READ_STATE],
        "authors": [author.to_hex()],
        "read_state_snapshot": 1,
    });
    if filters != [expected] {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "read_state_snapshot requires one exact version-1 own-author kind-30078 filter",
        ));
    }
    let snapshot = state
        .db
        .read_state_snapshot(tenant.community(), author)
        .await
        .map_err(|error| match error {
            buzz_db::DbError::ReadStateSnapshotTooLarge => api_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "read_state_snapshot exceeds event or byte limit; cannot prove complete",
            ),
            _ => {
                tracing::error!(%error, "read-state snapshot failed");
                api_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "read_state_snapshot unavailable; cannot prove complete",
                )
            }
        })?;
    Ok(Json(json!({
        "read_state_snapshot": 1,
        "complete": true,
        "community_id": tenant.community().as_uuid(),
        "pubkey": author.to_hex(),
        "snapshot_id": snapshot.snapshot_id,
        "events": snapshot.events,
    })))
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn request(
        state: Arc<AppState>,
        host: &str,
        key: Option<&Keys>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method("POST")
            .uri("/query")
            .header("host", host);
        if let Some(key) = key {
            req = req.header("x-pubkey", key.public_key().to_hex());
        }
        let response = crate::router::build_router(state)
            .oneshot(
                req.body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 10 * 1024 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    fn filter(key: &Keys) -> Value {
        json!([{"kinds":[30078], "authors":[key.public_key().to_hex()], "read_state_snapshot":1}])
    }

    async fn setup() -> (Arc<AppState>, String, buzz_core::CommunityId, Keys) {
        let state = super::super::postgres_tests::bridge_handler_test_state()
            .await
            .expect("isolated Postgres/Redis fixture");
        let host = format!("snapshot-{}.local", uuid::Uuid::new_v4().simple());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .unwrap()
            .id;
        (state, host, community, Keys::generate())
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn read_state_snapshot_router_real_auth_membership_and_replay() {
        use base64::Engine;
        use sha2::{Digest, Sha256};

        let (fixture, host, community, key) = setup().await;
        let mut state = (*fixture).clone();
        let config = Arc::make_mut(&mut state.config);
        config.require_auth_token = true;
        config.require_relay_membership = true;
        // Restore the production Redis guard; never accept every replay in this test.
        state.nip98_replay = Arc::new(buzz_pubsub::RedisNip98ReplayGuard::new(
            state.redis_pool.clone(),
        ));
        let state = Arc::new(state);
        let body = serde_json::to_vec(&filter(&key)).unwrap();
        let proof = |signed_host: &str| {
            let auth = EventBuilder::new(Kind::Custom(27235), "")
                .tags([
                    Tag::parse(["u", &format!("https://{signed_host}/query")]).unwrap(),
                    Tag::parse(["method", "POST"]).unwrap(),
                    Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
                    Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
                ])
                .sign_with_keys(&key)
                .unwrap();
            format!(
                "Nostr {}",
                base64::engine::general_purpose::STANDARD
                    .encode(serde_json::to_vec(&auth).unwrap())
            )
        };
        let post = |authorization: String, bytes: Vec<u8>| {
            let router = crate::router::build_router(state.clone());
            let req = Request::builder()
                .method("POST")
                .uri("/query")
                .header("host", &host)
                .header("authorization", authorization)
                .body(Body::from(bytes))
                .unwrap();
            async move {
                let response = router.oneshot(req).await.unwrap();
                let status = response.status();
                let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
                (status, serde_json::from_slice::<Value>(&bytes).unwrap())
            }
        };
        let denied = post(proof(&host), body.clone()).await;
        assert_eq!(denied.0, StatusCode::FORBIDDEN, "{}", denied.1);
        assert_eq!(denied.1["error"], "relay_membership_required");
        assert_ne!(denied.1["complete"], true);

        state
            .db
            .add_relay_member(community, &key.public_key().to_hex(), "member", None)
            .await
            .unwrap();
        // Dev headers must not bypass real auth even for an admitted member.
        assert_eq!(
            request(state.clone(), &host, Some(&key), filter(&key))
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            post(proof("wrong-host.invalid"), body.clone()).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            post(proof(&host), b"[]".to_vec()).await.0,
            StatusCode::UNAUTHORIZED
        );

        let auth = proof(&host);
        let accepted = post(auth.clone(), body.clone()).await;
        assert_eq!(accepted.0, StatusCode::OK, "{}", accepted.1);
        assert_eq!(accepted.1["complete"], true);
        assert_eq!(accepted.1["community_id"], community.as_uuid().to_string());
        let replay = post(auth, body.clone()).await;
        assert_eq!(replay.0, StatusCode::UNAUTHORIZED, "{}", replay.1);
        assert_ne!(replay.1["complete"], true);
        assert_eq!(post(proof(&host), body).await.0, StatusCode::OK);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn read_state_snapshot_router_scope_discovery_and_strict_contract() {
        let (state, host, community, key) = setup().await;
        let other = Keys::generate();
        let other_host = format!("other-{}.local", uuid::Uuid::new_v4().simple());
        let other_community = state
            .db
            .ensure_configured_community(&other_host)
            .await
            .unwrap()
            .id;
        let event = EventBuilder::new(Kind::Custom(30078), "encrypted")
            .tags([Tag::identifier("unrelated-app-without-t-tag")])
            .sign_with_keys(&key)
            .unwrap();
        state
            .db
            .replace_parameterized_event(community, &event, "unrelated-app-without-t-tag", None)
            .await
            .unwrap();
        let other_event = EventBuilder::new(Kind::Custom(30078), "other-author")
            .tags([Tag::identifier("other")])
            .sign_with_keys(&other)
            .unwrap();
        state
            .db
            .replace_parameterized_event(community, &other_event, "other", None)
            .await
            .unwrap();
        state
            .db
            .replace_parameterized_event(
                other_community,
                &event,
                "unrelated-app-without-t-tag",
                None,
            )
            .await
            .unwrap();

        let (status, body) = request(state.clone(), &host, Some(&key), filter(&key)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["complete"], true);
        assert_eq!(body["read_state_snapshot"], 1);
        assert_eq!(body["community_id"], community.as_uuid().to_string());
        assert_eq!(body["pubkey"], key.public_key().to_hex());
        assert_eq!(body["events"], json!([event]));
        let info = crate::nip11::nip11_document(&state, &host).await;
        assert_eq!(
            info.read_state_snapshot.as_ref().unwrap()["community_id"],
            body["community_id"]
        );
        assert!(crate::nip11::nip11_document(&state, "unmapped.invalid")
            .await
            .read_state_snapshot
            .is_none());
        let (_, other_body) = request(state.clone(), &other_host, Some(&key), filter(&key)).await;
        assert_eq!(
            other_body["community_id"],
            other_community.as_uuid().to_string()
        );
        assert_ne!(other_body["snapshot_id"], body["snapshot_id"]);
        let (status, _) = request(state.clone(), &host, None, filter(&key)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) =
            request(state.clone(), "unmapped.invalid", Some(&key), filter(&key)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = request(state.clone(), &host, Some(&other), filter(&key)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        for (name, value) in [
            ("since", json!(0)),
            ("limit", json!(1)),
            ("#t", json!(["read-state"])),
            ("read_state_snapshot", json!(2)),
            ("read_state_snapshot", json!(false)),
            ("read_state_snapshot", Value::Null),
        ] {
            let mut bad = filter(&key);
            bad[0][name] = value;
            let (status, body) = request(state.clone(), &host, Some(&key), bad).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {body}");
            assert_ne!(body["complete"], true);
        }
        let mut mixed = filter(&key);
        mixed.as_array_mut().unwrap().push(json!({"kinds":[0]}));
        assert_eq!(
            request(state.clone(), &host, Some(&key), mixed).await.0,
            StatusCode::BAD_REQUEST
        );
        let ordinary = json!([{"kinds":[30078], "authors":[key.public_key().to_hex()]}]);
        let (status, body) = request(state, &host, Some(&key), ordinary).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.is_array());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn read_state_snapshot_router_corruption_and_storage_failure_are_not_complete() {
        let (state, host, community, key) = setup().await;
        let event = EventBuilder::new(Kind::Custom(30078), "payload")
            .tags([Tag::identifier("fixture")])
            .sign_with_keys(&key)
            .unwrap();
        state
            .db
            .replace_parameterized_event(community, &event, "fixture", None)
            .await
            .unwrap();
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        for malformed in [json!(42), json!([["d", "fixture"]])] {
            sqlx::query(
                "UPDATE events SET tags=$1, content='corrupt' WHERE community_id=$2 AND id=$3",
            )
            .bind(malformed)
            .bind(community.as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .execute(&pool)
            .await
            .unwrap();
            let (status, body) = request(state.clone(), &host, Some(&key), filter(&key)).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
            assert_ne!(body["complete"], true);
            assert!(body.get("events").is_none());
        }
        // Real DB error, not a mocked empty query: lock out the relation in this
        // isolated per-test DB, while tenant/auth lookups remain usable.
        sqlx::query("ALTER TABLE events RENAME TO snapshot_unavailable_events")
            .execute(&pool)
            .await
            .unwrap();
        let (status, body) = request(state, &host, Some(&key), filter(&key)).await;
        sqlx::query("ALTER TABLE snapshot_unavailable_events RENAME TO events")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert_ne!(body["complete"], true);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn read_state_snapshot_router_beyond_page_cap_and_overflow() {
        let (state, host, community, key) = setup().await;
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        // Same-second plateau > ordinary 1,000 cap, including unrelated data.
        for n in 0..1001 {
            let event = EventBuilder::new(Kind::Custom(30078), "payload")
                .custom_created_at(nostr::Timestamp::from(1789090000))
                .tags([Tag::identifier(format!("fixture-{n}"))])
                .sign_with_keys(&key)
                .unwrap();
            state
                .db
                .insert_event(community, &event, None)
                .await
                .unwrap();
        }
        let (status, body) = request(state.clone(), &host, Some(&key), filter(&key)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["events"].as_array().unwrap().len(), 1001);
        // Stored byte preflight must reject before decoding these corrupt rows.
        sqlx::query("UPDATE events SET content=repeat('x', 9000) WHERE community_id=$1")
            .bind(community.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
        let (status, body) = request(state.clone(), &host, Some(&key), filter(&key)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
        assert_ne!(body["complete"], true);
        sqlx::query("DELETE FROM events WHERE community_id=$1")
            .bind(community.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
        // Count overflow must not masquerade as a capped complete response.
        sqlx::query("INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig)
            SELECT $1, decode(md5(n::text)||md5(n::text),'hex'),$2,to_timestamp(1789090000),30078,'[]','',decode(repeat('00',64),'hex')
            FROM generate_series(1,4097) n")
            .bind(community.as_uuid()).bind(key.public_key().to_bytes().to_vec()).execute(&pool).await.unwrap();
        let (status, body) = request(state, &host, Some(&key), filter(&key)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
        assert_ne!(body["complete"], true);
    }
}
