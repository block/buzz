//! Tests for the staff-only community reads and the HEAD binding of every
//! admin read. The unsigned, disabled-auth, HEAD and validation checks reject
//! or answer before any database access; the rest are `#[ignore]`d and run in
//! the PostgreSQL lane.

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
    disabled_mode_state, make_nostr_auth, make_nostr_auth_raw_tags, nip98_state,
    nip98_state_with_real_pool, test_operator_keys,
};
use super::super::router;
use crate::state::AppState;
use crate::test_support::database_url;

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

/// A request to `sent` whose credential was signed for `signed_method` and
/// `signed_uri`, so tests can tamper with the method or the query.
fn signed_as(
    keys: &nostr::Keys,
    method: &str,
    signed_method: &str,
    signed_uri: &str,
    sent: &str,
) -> Request<Body> {
    let url = format!("https://admin.example{ADMIN_API_PREFIX}{signed_uri}");
    let tags = vec![
        nostr::Tag::parse(["u", &url]).unwrap(),
        nostr::Tag::parse(["method", signed_method]).unwrap(),
    ];
    Request::builder()
        .method(method)
        .uri(sent)
        .header(header::HOST, "admin.example")
        .header(header::AUTHORIZATION, make_nostr_auth_raw_tags(keys, tags))
        .body(Body::empty())
        .unwrap()
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

/// Every legacy GET read route, the status a HEAD-signed HEAD gets past auth,
/// and whether producing that status needs the database.
fn legacy_reads() -> Vec<(String, StatusCode, bool)> {
    let id = Uuid::new_v4();
    vec![
        ("/probe".to_owned(), StatusCode::OK, false),
        (
            "/reports?status=bogus".to_owned(),
            StatusCode::BAD_REQUEST,
            false,
        ),
        (format!("/reports/{id}"), StatusCode::NOT_FOUND, true),
        ("/feedback".to_owned(), StatusCode::OK, true),
        (format!("/feedback/{id}"), StatusCode::NOT_FOUND, true),
        (
            format!("/feedback/{id}/attachments/not-a-hash"),
            StatusCode::NOT_FOUND,
            false,
        ),
        ("/operators".to_owned(), StatusCode::OK, true),
        (
            "/members/restrictions?communityHost=a.example&limit=0".to_owned(),
            StatusCode::BAD_REQUEST,
            false,
        ),
    ]
}

/// Axum serves HEAD through GET handlers; the credential must be checked
/// against the real method, so a GET-signed HEAD is refused on every legacy
/// read, and a HEAD-signed HEAD passes auth wherever the answer needs no
/// database (the rest are covered by the Postgres lane below).
#[tokio::test]
async fn legacy_reads_authorize_head_with_the_real_method() {
    let keys = test_operator_keys();
    let state = nip98_state(vec![keys.public_key().to_hex()]).await;
    for (uri, want, needs_db) in legacy_reads() {
        let head = |signed: &str| signed_as(&keys, "HEAD", signed, &uri, &uri);
        assert_eq!(
            send(&state, head("GET")).await.0,
            StatusCode::UNAUTHORIZED,
            "GET-signed HEAD {uri}"
        );
        if !needs_db {
            assert_eq!(send(&state, head("HEAD")).await.0, want, "HEAD {uri}");
        }
    }
}

/// The credential covers the full target: a GET-signed HEAD, or a query
/// changed after signing, is refused on every new route.
#[tokio::test]
async fn new_reads_reject_method_and_query_tampering() {
    let keys = test_operator_keys();
    let state = nip98_state(vec![keys.public_key().to_hex()]).await;
    let mut routes = new_routes("a.example");
    routes[0] = "/communities?q=a.example".to_owned();
    for uri in routes {
        let head = signed_as(&keys, "HEAD", "GET", &uri, &uri);
        assert_eq!(
            send(&state, head).await.0,
            StatusCode::UNAUTHORIZED,
            "{uri}"
        );
        let tampered = uri.replace("a.example", "b.example");
        let request = signed_as(&keys, "GET", "GET", &uri, &tampered);
        assert_eq!(
            send(&state, request).await.0,
            StatusCode::UNAUTHORIZED,
            "{tampered}"
        );
    }
}

/// Out-of-range limits are refused before any database access.
#[tokio::test]
async fn community_reads_reject_out_of_range_limits() {
    let keys = test_operator_keys();
    let state = nip98_state(vec![keys.public_key().to_hex()]).await;
    for uri in [
        "/communities?limit=0",
        "/communities?limit=101",
        "/members/search?communityHost=a.example&q=a&limit=0",
        "/members/search?communityHost=a.example&q=a&limit=51",
    ] {
        let (status, body) = get(&state, &keys, uri).await;
        assert_eq!(
            (status, body["error"]["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_limit")),
            "{uri}"
        );
    }
}

#[tokio::test]
async fn member_search_rejects_empty_and_overlong_queries() {
    let keys = test_operator_keys();
    let state = nip98_state(vec![keys.public_key().to_hex()]).await;
    for q in ["%20%20".to_owned(), "a".repeat(101)] {
        let uri = format!("/members/search?communityHost=a.example&q={q}");
        let (status, body) = get(&state, &keys, &uri).await;
        assert_eq!(
            (status, body["error"]["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_query")),
            "{uri}"
        );
    }
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
            let head = signed_as(&keys, "HEAD", "HEAD", uri, uri);
            assert_eq!(send(&state, head).await.0, want, "HEAD {uri}");
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
    // The page cap turns a cursor that never ends into a failure, not a hang.
    let mut seen = Vec::new();
    let mut next = Some(format!("/communities?q={}&limit=2", p.to_uppercase()));
    for _ in 0..=live.len() {
        let Some(uri) = next.take() else { break };
        let (status, page) = get(&state, &keys, &uri).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        seen.extend(hosts(&page));
        next = page["nextCursor"]
            .as_str()
            .map(|cursor| format!("/communities?q={p}&limit=2&cursor={cursor}"));
    }
    assert!(next.is_none(), "paging did not end: {next:?}");
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

/// Only the staff-roster read fails: each connection shadows
/// `relay_operators` with an incompatible temp table, while the community,
/// profile, role and restriction reads still succeed. The route must answer
/// 500, never `isStaff: false`.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn member_lookup_fails_closed_when_the_staff_lookup_fails() {
    let (pool, _) = fixture().await;
    let host = unique_host("staff-fail");
    community(&pool, &host).await;
    let broken = sqlx::postgres::PgPoolOptions::new()
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("CREATE TEMP TABLE relay_operators (unrelated int)")
                    .execute(conn)
                    .await
                    .map(|_| ())
            })
        })
        .connect(&database_url())
        .await
        .expect("connect");
    let state = nip98_state_with_real_pool(broken).await;
    let (status, body) = get(
        &state,
        &test_operator_keys(),
        &format!("/members/{PK}?communityHost={host}"),
    )
    .await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (StatusCode::INTERNAL_SERVER_ERROR, Some("internal_error")),
        "{body}"
    );
}

/// The boundary limits return exactly that many matches.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn member_search_returns_up_to_the_limit() {
    let (pool, state) = fixture().await;
    let keys = test_operator_keys();
    let host = unique_host("search-limit");
    let c = community(&pool, &host).await;
    let tag = Uuid::new_v4().simple().to_string();
    for i in 0..51u8 {
        seed_profile(&pool, c, &[i; 32], &format!("{tag} {i}")).await;
    }
    for limit in [1usize, 50] {
        let uri = format!("/members/search?communityHost={host}&q={tag}&limit={limit}");
        let (status, page) = get(&state, &keys, &uri).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(
            page["items"].as_array().unwrap().len(),
            limit,
            "limit={limit}"
        );
    }
}

/// The database-backed half of
/// `legacy_reads_authorize_head_with_the_real_method`.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn legacy_reads_serve_a_head_signed_head() {
    let (_, state) = fixture().await;
    let keys = test_operator_keys();
    for (uri, want, _) in legacy_reads().into_iter().filter(|r| r.2) {
        let head = signed_as(&keys, "HEAD", "HEAD", &uri, &uri);
        assert_eq!(send(&state, head).await.0, want, "HEAD {uri}");
    }
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
