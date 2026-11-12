use super::postgres_tests::{
    operator_test_state, read_json, signed_operator_request, INGRESS_HOST,
};
use axum::http::StatusCode;
use buzz_core::CommunityId;
use nostr::{EventBuilder, Keys, Kind, Tag};
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn listener_thread_route_requires_both_listener_auth_and_target_scoped_consent() {
    let listener = Keys::generate();
    let target = Keys::generate();
    let outsider = Keys::generate();
    let mut state = operator_test_state(&[])
        .await
        .expect("local operator router state");
    let config = Arc::make_mut(&mut Arc::get_mut(&mut state).unwrap().config);
    config.operator_listener_delivery_urls.insert(
        listener.public_key().to_hex(),
        url::Url::parse("http://listener.example/deliver").unwrap(),
    );
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    let pool = state.db.pool();
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(community)
        .bind(format!("http-watch-{community}.example"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users(community_id,pubkey) VALUES($1,$2)")
        .bind(community)
        .bind(target.public_key().as_bytes().as_slice())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(community_id,id,name,created_by) VALUES($1,$2,'thread',$3)")
        .bind(community)
        .bind(channel)
        .bind(target.public_key().as_bytes().as_slice())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey) VALUES($1,$2,$3)")
        .bind(community)
        .bind(channel)
        .bind(target.public_key().as_bytes().as_slice())
        .execute(pool)
        .await
        .unwrap();
    state
        .db
        .register_operator_listener_pubkeys(
            listener.public_key().as_bytes(),
            &[target.public_key().to_bytes().to_vec()],
        )
        .await
        .unwrap();
    let root = EventBuilder::new(Kind::Custom(9), "root")
        .sign_with_keys(&target)
        .unwrap();
    buzz_db::event::insert_event(
        pool,
        CommunityId::from_uuid(community),
        &root,
        Some(channel),
    )
    .await
    .unwrap();
    let binding = serde_json::json!({"community_id":community,"channel_id":channel,"root_event_id":root.id.to_hex()});
    let proof = |url: &str| {
        EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_OPERATOR_LISTENER_REGISTRATION as u16),
            binding.to_string(),
        )
        .tags([
            Tag::public_key(listener.public_key()),
            Tag::parse(["u", url]).unwrap(),
        ])
        .sign_with_keys(&target)
        .unwrap()
    };
    let path = "/operator/listener/threads";
    let body =
        serde_json::json!({"registrations":[proof(&format!("http://{INGRESS_HOST}{path}"))]})
            .to_string();
    let denied =
        signed_operator_request(state.clone(), &outsider, "POST", path, Some(body.clone())).await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let wrong=serde_json::json!({"registrations":[proof("http://wrong.example/operator/listener/threads")]}).to_string();
    let denied = signed_operator_request(state.clone(), &listener, "POST", path, Some(wrong)).await;
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);
    let response =
        signed_operator_request(state.clone(), &listener, "POST", path, Some(body)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(read_json(response).await["expires_after_seconds"], 2592000);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operator_listener_threads WHERE community_id=$1")
            .bind(community)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    let removal=serde_json::json!({"community_id":community,"channel_id":channel,"root_event_id":root.id.to_hex(),"pubkey":target.public_key().to_hex()}).to_string();
    let removed =
        signed_operator_request(state.clone(), &listener, "DELETE", path, Some(removal)).await;
    assert_eq!(removed.status(), StatusCode::OK);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operator_listener_threads WHERE community_id=$1")
            .bind(community)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}
