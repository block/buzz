//! Tests for the staff-only community reads. The unsigned, disabled-auth and
//! HEAD checks reject or answer before any database access; the rest are
//! `#[ignore]`d and run in the PostgreSQL lane.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use buzz_core::CommunityId;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

use super::super::auth::ADMIN_API_PREFIX;
use super::super::postgres_tests::{
    database_url, disabled_mode_state, make_nostr_auth, make_nostr_auth_raw_tags, nip98_state,
    nip98_state_with_real_pool, test_operator_keys,
};
use super::super::router;
use crate::state::AppState;

const PK: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn new_routes(host: &str) -> Vec<String> {
    vec![
        "/communities".to_string(),
        format!("/members/search?communityHost={host}&q=a"),
        format!("/members/{PK}?communityHost={host}"),
        format!("/events/{PK}?communityHost={host}"),
    ]
}

async fn send(state: &Arc<AppState>, request: Request<Body>) -> (StatusCode, Value) {
    let response = router(state.clone())
        .oneshot(request)
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn signed_get(keys: &nostr::Keys, uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(header::HOST, "admin.example")
        .header(header::AUTHORIZATION, make_nostr_auth(keys, uri))
        .body(Body::empty())
        .expect("request")
}

async fn get(state: &Arc<AppState>, keys: &nostr::Keys, uri: &str) -> (StatusCode, Value) {
    send(state, signed_get(keys, uri)).await
}

#[tokio::test]
async fn new_reads_reject_unsigned_requests() {
    let state = nip98_state(vec![test_operator_keys().public_key().to_hex()]).await;
    for uri in new_routes("a.example") {
        let request = Request::builder()
            .uri(&uri)
            .header(header::HOST, "admin.example")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            send(&state, request).await.0,
            StatusCode::UNAUTHORIZED,
            "{uri}"
        );
    }
}

#[tokio::test]
async fn new_reads_refuse_disabled_auth_mode() {
    let state = disabled_mode_state().await;
    for uri in new_routes("a.example") {
        let request = Request::builder()
            .uri(&uri)
            .header(header::HOST, "admin.example")
            .body(Body::empty())
            .unwrap();
        let (status, body) = send(&state, request).await;
        assert_eq!(
            (status, body["error"]["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("forbidden")),
            "{uri}"
        );
    }
}

/// Axum serves HEAD through GET handlers; the credential must be checked
/// against the real method, so a HEAD-signed HEAD passes and a GET-signed
/// HEAD does not.
#[tokio::test]
async fn admin_reads_authorize_head_with_the_real_method() {
    let keys = test_operator_keys();
    let state = nip98_state(vec![keys.public_key().to_hex()]).await;
    let head = |signed: &str| {
        let url = format!("https://admin.example{ADMIN_API_PREFIX}/probe");
        let tags = vec![
            nostr::Tag::parse(["u", &url]).unwrap(),
            nostr::Tag::parse(["method", signed]).unwrap(),
        ];
        Request::builder()
            .method("HEAD")
            .uri("/probe")
            .header(header::HOST, "admin.example")
            .header(header::AUTHORIZATION, make_nostr_auth_raw_tags(&keys, tags))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(send(&state, head("HEAD")).await.0, StatusCode::OK);
    assert_eq!(send(&state, head("GET")).await.0, StatusCode::UNAUTHORIZED);
}

async fn community(pool: &sqlx::PgPool, host: &str) -> CommunityId {
    buzz_db::Db::from_pool(pool.clone())
        .ensure_configured_community(host)
        .await
        .expect("create community")
        .id
}

async fn seed_profile(pool: &sqlx::PgPool, c: CommunityId, pubkey: &[u8], name: &str) {
    sqlx::query(
        "INSERT INTO users (community_id, pubkey, display_name, about) VALUES ($1, $2, $3, $3)",
    )
    .bind(c.as_uuid())
    .bind(pubkey)
    .bind(name)
    .execute(pool)
    .await
    .expect("seed profile");
}

/// A raw event row: the same id may be stored in two communities with
/// different content, which is exactly what isolation must not leak.
async fn seed_event(pool: &sqlx::PgPool, c: CommunityId, id: &[u8], content: &str, deleted: bool) {
    sqlx::query(
        "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, deleted_at) \
         VALUES ($1, $2, $2, now(), 1, '[]', $3, $2, CASE WHEN $4 THEN now() END)",
    )
    .bind(c.as_uuid())
    .bind(id)
    .bind(content)
    .bind(deleted)
    .execute(pool)
    .await
    .expect("seed event");
}

async fn fixture() -> (sqlx::PgPool, Arc<AppState>) {
    let pool = sqlx::PgPool::connect(&database_url())
        .await
        .expect("connect");
    let state = nip98_state_with_real_pool(pool.clone()).await;
    (pool, state)
}

fn unique_host(label: &str) -> String {
    format!("{label}-{}.example", Uuid::new_v4().simple())
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn new_reads_serve_operators_and_moderators_and_refuse_non_staff() {
    let (pool, state) = fixture().await;
    let host = unique_host("reads-auth");
    community(&pool, &host).await;
    let moderator = nostr::Keys::generate();
    buzz_db::Db::from_pool(pool.clone())
        .upsert_relay_operator(
            &moderator.public_key().to_bytes(),
            "moderator",
            &[1u8; 32],
            true,
        )
        .await
        .expect("seed moderator");
    let expected = [
        StatusCode::OK,
        StatusCode::OK,
        StatusCode::OK,
        StatusCode::NOT_FOUND,
    ];
    for (uri, want) in new_routes(&host).iter().zip(expected) {
        for keys in [test_operator_keys(), moderator.clone()] {
            assert_eq!(get(&state, &keys, uri).await.0, want, "{uri}");
        }
        let stranger = nostr::Keys::generate();
        assert_eq!(
            get(&state, &stranger, uri).await.0,
            StatusCode::FORBIDDEN,
            "{uri}"
        );
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn community_reads_never_return_another_communitys_data() {
    let (pool, state) = fixture().await;
    let keys = test_operator_keys();
    let (host_a, host_b) = (unique_host("iso-a"), unique_host("iso-b"));
    let (a, b) = (
        community(&pool, &host_a).await,
        community(&pool, &host_b).await,
    );

    let shared = [0x31u8; 32];
    let foreign = [0x32u8; 32];
    let tag = Uuid::new_v4().simple().to_string();
    seed_profile(&pool, a, &shared, &format!("{tag} in a")).await;
    seed_profile(&pool, b, &shared, &format!("{tag} in b")).await;
    seed_profile(&pool, b, &foreign, &format!("{tag} only b")).await;
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'admin')")
        .bind(b.as_uuid())
        .bind(hex::encode(shared))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO community_bans (community_id, pubkey, banned, actor_pubkey) VALUES ($1, $2, true, $2)")
        .bind(b.as_uuid())
        .bind(shared.as_slice())
        .execute(&pool)
        .await
        .unwrap();
    let (shared_event, foreign_event) = ([0x41u8; 32], [0x42u8; 32]);
    seed_event(&pool, a, &shared_event, "content in a", false).await;
    seed_event(&pool, b, &shared_event, "content in b", false).await;
    seed_event(&pool, b, &foreign_event, "only in b", false).await;

    let (status, found) = get(
        &state,
        &keys,
        &format!("/members/search?communityHost={host_a}&q={tag}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    let names: Vec<&str> = found["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["displayName"].as_str().unwrap())
        .collect();
    assert_eq!(names, [format!("{tag} in a")]);

    let (_, member) = get(
        &state,
        &keys,
        &format!("/members/{}?communityHost={host_a}", hex::encode(shared)),
    )
    .await;
    assert_eq!(member["profile"]["displayName"], format!("{tag} in a"));
    assert_eq!(
        (member["role"].clone(), member["banned"].clone()),
        (Value::Null, false.into())
    );

    let (status, stranger) = get(
        &state,
        &keys,
        &format!("/members/{}?communityHost={host_a}", hex::encode(foreign)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (stranger["profile"].clone(), stranger["role"].clone()),
        (Value::Null, Value::Null)
    );

    let (_, event) = get(
        &state,
        &keys,
        &format!(
            "/events/{}?communityHost={host_a}",
            hex::encode(shared_event)
        ),
    )
    .await;
    assert_eq!(event["content"], "content in a");
    let (status, missing) = get(
        &state,
        &keys,
        &format!(
            "/events/{}?communityHost={host_a}",
            hex::encode(foreign_event)
        ),
    )
    .await;
    assert_eq!(
        (status, missing["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("event_not_found"))
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn community_directory_pages_live_hosts_by_literal_prefix() {
    let (pool, state) = fixture().await;
    let keys = test_operator_keys();
    let p = format!("dir{}", Uuid::new_v4().simple());
    let live = [
        format!("{p}-a.example"),
        format!("{p}-b.example"),
        format!("{p}-c.example"),
        format!("{p}%lit.example"),
        format!("{p}_u.example"),
    ];
    for host in &live {
        sqlx::query("INSERT INTO communities (host) VALUES ($1)")
            .bind(host)
            .execute(&pool)
            .await
            .unwrap();
    }
    for (host, archived, state_, deleted) in [
        (format!("{p}-arch.example"), true, "active", false),
        (format!("{p}-del.example"), false, "quiescing", false),
        (format!("{p}-gone.example"), false, "tombstone", true),
    ] {
        sqlx::query(
            "INSERT INTO communities (host, archived_at, deletion_state, deleted_at) \
             VALUES ($1, CASE WHEN $2 THEN now() END, $3, CASE WHEN $4 THEN now() END)",
        )
        .bind(host)
        .bind(archived)
        .bind(state_)
        .bind(deleted)
        .execute(&pool)
        .await
        .unwrap();
    }

    let hosts = |page: &Value| -> Vec<String> {
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["host"].as_str().unwrap().to_owned())
            .collect()
    };
    // Paging with limit 2 visits every live host once, then stops.
    let mut seen = Vec::new();
    let mut uri = format!("/communities?q={}&limit=2", p.to_uppercase());
    loop {
        let (status, page) = get(&state, &keys, &uri).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        seen.extend(hosts(&page));
        match page["nextCursor"].as_str() {
            Some(cursor) => uri = format!("/communities?q={p}&limit=2&cursor={cursor}"),
            None => break,
        }
    }
    let mut want = live.to_vec();
    want.sort();
    seen.sort();
    assert_eq!(seen, want);

    // `%` and `_` match themselves, not any character.
    let (_, pct) = get(&state, &keys, &format!("/communities?q={p}%25")).await;
    assert_eq!(hosts(&pct), [format!("{p}%lit.example")]);
    let (_, under) = get(&state, &keys, &format!("/communities?q={p}_")).await;
    assert_eq!(hosts(&under), [format!("{p}_u.example")]);

    let (status, bad) = get(&state, &keys, "/communities?cursor=not-a-cursor").await;
    assert_eq!(
        (status, bad["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_cursor"))
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn member_lookup_reports_staff_and_active_restrictions() {
    let (pool, state) = fixture().await;
    let keys = test_operator_keys();
    let host = unique_host("lookup");
    let c = community(&pool, &host).await;
    let target = [0x51u8; 32];
    sqlx::query(
        "INSERT INTO community_bans (community_id, pubkey, banned, muted_until, actor_pubkey) \
         VALUES ($1, $2, true, now() + interval '1 hour', $2)",
    )
    .bind(c.as_uuid())
    .bind(target.as_slice())
    .execute(&pool)
    .await
    .unwrap();
    let (_, member) = get(
        &state,
        &keys,
        &format!("/members/{}?communityHost={host}", hex::encode(target)),
    )
    .await;
    assert_eq!(
        (member["banned"].clone(), member["isStaff"].clone()),
        (true.into(), false.into())
    );
    assert!(member["mutedUntil"].is_string(), "{member}");

    let staff = keys.public_key().to_hex();
    let (_, member) = get(
        &state,
        &keys,
        &format!("/members/{staff}?communityHost={host}"),
    )
    .await;
    assert_eq!(member["isStaff"], true);
    assert_eq!(member["pubkey"], staff);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn member_staff_lookup_fails_closed() {
    let (pool, state) = fixture().await;
    pool.close().await;
    let err = super::is_staff(&state, &[0x11u8; 32]).await.unwrap_err();
    assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn event_preview_reports_the_stored_event_and_its_deletion() {
    let (pool, state) = fixture().await;
    let keys = test_operator_keys();
    let host = unique_host("preview");
    let c = community(&pool, &host).await;
    let id = [0x61u8; 32];
    seed_event(&pool, c, &id, "deleted body", true).await;
    let (status, event) = get(
        &state,
        &keys,
        &format!("/events/{}?communityHost={host}", hex::encode(id)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{event}");
    assert_eq!(
        (
            event["id"].as_str(),
            event["authorPubkey"].as_str(),
            event["kind"].as_i64()
        ),
        (
            Some(hex::encode(id).as_str()),
            Some(hex::encode(id).as_str()),
            Some(1)
        )
    );
    assert!(
        event["deletedAt"].is_string() && event["channelId"].is_null(),
        "{event}"
    );
}
