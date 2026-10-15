//! Tests for the community reads, the HEAD binding of every admin read, and
//! the view/act/operator classification of every admin route. The unsigned, HEAD
//! and validation checks reject or answer before any database access; the rest
//! are `#[ignore]`d and run in the PostgreSQL lane.

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

fn community_read_routes(host: &str) -> Vec<String> {
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
async fn community_reads_reject_unsigned_requests() {
    let state = nip98_state(vec![test_operator_keys().public_key().to_hex()]).await;
    for uri in community_read_routes("a.example") {
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

/// The report, feedback, staffing and restriction GET routes, the status a
/// HEAD-signed HEAD gets past auth, and whether producing that status needs
/// the database.
fn moderation_reads() -> Vec<(String, StatusCode, bool)> {
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
/// against the real method, so a GET-signed HEAD is refused on every
/// one of these reads, and a HEAD-signed HEAD passes auth wherever the answer needs no
/// database (the rest are covered by the Postgres lane below).
#[tokio::test]
async fn moderation_reads_authorize_head_with_the_real_method() {
    let keys = test_operator_keys();
    let state = nip98_state(vec![keys.public_key().to_hex()]).await;
    for (uri, want, needs_db) in moderation_reads() {
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
/// changed after signing, is refused on every community read.
#[tokio::test]
async fn community_reads_reject_method_and_query_tampering() {
    let keys = test_operator_keys();
    let state = nip98_state(vec![keys.public_key().to_hex()]).await;
    let mut routes = community_read_routes("a.example");
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

/// `q` longer than a host can be is refused before any database access.
#[tokio::test]
async fn community_directory_rejects_a_query_longer_than_a_host() {
    let keys = test_operator_keys();
    let state = nip98_state(vec![keys.public_key().to_hex()]).await;
    let uri = format!("/communities?q={}", "a".repeat(256));
    let (status, body) = get(&state, &keys, &uri).await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_query"))
    );
}

/// A malformed event id gets the same answer as an absent event.
#[tokio::test]
async fn event_preview_answers_a_malformed_id_as_not_found() {
    let keys = test_operator_keys();
    let state = nip98_state(vec![keys.public_key().to_hex()]).await;
    for id in ["zz".repeat(32), "ab".repeat(31), "ab".repeat(33)] {
        let uri = format!("/events/{id}?communityHost=a.example");
        let (status, body) = get(&state, &keys, &uri).await;
        assert_eq!(
            (status, body["error"]["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("event_not_found")),
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

/// A raw kind-1 event row: the same id may be stored in two communities with
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
async fn community_reads_serve_operators_and_moderators_and_refuse_non_staff() {
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
    for (uri, want) in community_read_routes(&host).iter().zip(expected) {
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
    let (_, member) = get(
        &state,
        &keys,
        &format!("/members/{}?communityHost={host_b}", hex::encode(shared)),
    )
    .await;
    assert_eq!(
        (member["role"].clone(), member["banned"].clone()),
        ("admin".into(), true.into()),
        "{member}"
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

    let (status, longest) = get(
        &state,
        &keys,
        &format!("/communities?q={}", "a".repeat(255)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{longest}");

    // Not base64, base64 of nothing, and base64 of invalid UTF-8.
    for cursor in ["not-a-cursor!", "", "_w"] {
        let (status, bad) = get(&state, &keys, &format!("/communities?cursor={cursor}")).await;
        assert_eq!(
            (status, bad["error"]["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_cursor")),
            "cursor={cursor:?}"
        );
    }
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

/// A Moderator gets the staff answer too (the desktop preflights direct
/// actions with it); disabled mode has no signed caller and gets `null`.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn member_lookup_reports_staff_only_to_a_signed_caller() {
    let (pool, state) = fixture().await;
    let host = unique_host("lookup-staff");
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
    let operator = test_operator_keys().public_key().to_hex();
    let lookup = |pubkey: &str| format!("/members/{pubkey}?communityHost={host}");

    for (pubkey, want) in [(operator.as_str(), true), (PK, false)] {
        let (status, member) = get(&state, &moderator, &lookup(pubkey)).await;
        assert_eq!(status, StatusCode::OK, "{member}");
        assert_eq!(member["isStaff"], want, "{pubkey}");
    }

    let open = disabled_mode_state().await;
    let request = Request::builder()
        .uri(lookup(&operator))
        .header(header::HOST, "admin.example")
        .body(Body::empty())
        .unwrap();
    let (status, member) = send(&open, request).await;
    assert_eq!(status, StatusCode::OK, "{member}");
    assert!(member["isStaff"].is_null(), "{member}");
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
/// `moderation_reads_authorize_head_with_the_real_method`.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn moderation_reads_serve_a_head_signed_head() {
    let (_, state) = fixture().await;
    let keys = test_operator_keys();
    for (uri, want, _) in moderation_reads().into_iter().filter(|r| r.2) {
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

/// Every author-only, result-gated, `#p`-gated and shared-gated event is
/// answered as absent, even to staff and even when shared; a message is not.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn event_preview_hides_every_private_kind() {
    use buzz_core::kind::{
        AUTHOR_ONLY_KINDS, P_GATED_KINDS, RESULT_GATED_KINDS, SHARED_GATED_KINDS,
    };
    let (pool, state) = fixture().await;
    let keys = test_operator_keys();
    let host = unique_host("preview-hidden");
    let c = community(&pool, &host).await;
    for (i, &kind) in AUTHOR_ONLY_KINDS
        .iter()
        .chain(RESULT_GATED_KINDS)
        .chain(P_GATED_KINDS)
        .chain(SHARED_GATED_KINDS)
        .enumerate()
    {
        let id = [0x80 + i as u8; 32];
        sqlx::query(
            "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig) \
             VALUES ($1, $2, $2, now(), $3, '[[\"shared\",\"true\"]]', 'hidden', $2)",
        )
        .bind(c.as_uuid())
        .bind(id)
        .bind(kind as i32)
        .execute(&pool)
        .await
        .expect("seed hidden event");
        let uri = format!("/events/{}?communityHost={host}", hex::encode(id));
        let (status, body) = get(&state, &keys, &uri).await;
        assert_eq!(
            (status, body["error"]["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("event_not_found")),
            "kind {kind}: {body}"
        );
    }

    let id = [0x7f_u8; 32];
    seed_event(&pool, c, &id, "visible", false).await;
    let uri = format!("/events/{}?communityHost={host}", hex::encode(id));
    let (status, body) = get(&state, &keys, &uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["content"], "visible", "{body}");
}

/// Every query struct refuses an unknown field, on reads and on a write; the
/// refused write leaves the ban in place.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn admin_queries_reject_unknown_fields() {
    let (pool, state) = fixture().await;
    let keys = test_operator_keys();
    let host = unique_host("unknown-field");
    let c = community(&pool, &host).await;
    let target = [0x91u8; 32];
    let db = buzz_db::Db::from_pool(pool.clone());
    db.ban_community_member(c, &target, &keys.public_key().to_bytes(), None, None)
        .await
        .expect("seed ban");

    for uri in [
        "/communities?bogus=1".to_owned(),
        format!("/members/search?communityHost={host}&q=a&bogus=1"),
        format!("/members/{PK}?communityHost={host}&bogus=1"),
    ] {
        assert_eq!(
            get(&state, &keys, &uri).await.0,
            StatusCode::BAD_REQUEST,
            "{uri}"
        );
    }

    let uri = format!(
        "/members/{}/ban?communityHost={host}&bogus=1",
        hex::encode(target)
    );
    let unban = signed_as(&keys, "DELETE", "DELETE", &uri, &uri);
    assert_eq!(
        send(&state, unban).await.0,
        StatusCode::BAD_REQUEST,
        "{uri}"
    );
    let restriction = db
        .moderation_restriction_state(c, &target)
        .await
        .expect("read ban");
    assert!(
        restriction.banned,
        "the refused unban must not lift the ban"
    );
}

/// Disabled mode serves the community reads to an unsigned caller, and a
/// query for one community still returns only that community's rows.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn community_reads_serve_disabled_mode_within_the_selected_community() {
    let (pool, _) = fixture().await;
    let state = disabled_mode_state().await;
    let (host_a, host_b) = (unique_host("open-a"), unique_host("open-b"));
    let (a, b) = (
        community(&pool, &host_a).await,
        community(&pool, &host_b).await,
    );
    let member = [0x71u8; 32];
    let event = [0x72u8; 32];
    let tag = Uuid::new_v4().simple().to_string();
    seed_profile(&pool, a, &member, &format!("{tag} in a")).await;
    seed_profile(&pool, b, &member, &format!("{tag} in b")).await;
    seed_event(&pool, a, &event, "content in a", false).await;
    seed_event(&pool, b, &event, "content in b", false).await;

    let state = &state;
    let unsigned = |uri: String| async move {
        let request = Request::builder()
            .uri(&uri)
            .header(header::HOST, "admin.example")
            .body(Body::empty())
            .unwrap();
        let (status, body) = send(state, request).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        body
    };

    let directory = unsigned(format!("/communities?q={host_b}")).await;
    assert_eq!(directory["items"][0]["host"], host_b.as_str());
    assert_eq!(directory["items"].as_array().unwrap().len(), 1);

    let found = unsigned(format!("/members/search?communityHost={host_b}&q={tag}")).await;
    let names: Vec<&str> = found["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["displayName"].as_str().unwrap())
        .collect();
    assert_eq!(names, [format!("{tag} in b")]);

    let profile = unsigned(format!(
        "/members/{}?communityHost={host_b}",
        hex::encode(member)
    ))
    .await;
    assert_eq!(profile["profile"]["displayName"], format!("{tag} in b"));

    let preview = unsigned(format!(
        "/events/{}?communityHost={host_b}",
        hex::encode(event)
    ))
    .await;
    assert_eq!(preview["content"], "content in b");
}

/// Which check a route makes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Check {
    /// Any access: signed staff in nip98, any caller in disabled mode.
    View,
    /// A signed staff member; refused in disabled mode.
    Act,
    /// A signed Operator; refused in disabled mode.
    Operator,
}

/// Every route the admin API mounts, with its check. A new route must be
/// added here with a deliberate policy: `every_route_is_listed` fails while
/// the number of `.route(` registrations in `router()` differs from this list.
fn every_route() -> Vec<(&'static str, String, Check)> {
    use Check::{Act, Operator, View};
    let id = Uuid::nil();
    let host = "communityHost=a.example";
    vec![
        ("GET", "/probe".into(), View),
        ("GET", "/reports".into(), View),
        ("GET", format!("/reports/{id}"), View),
        ("POST", format!("/reports/{id}/resolve"), Act),
        ("POST", format!("/reports/{id}/reopen"), Act),
        ("POST", format!("/reports/{id}/cancel"), Act),
        ("GET", "/feedback".into(), View),
        ("GET", format!("/feedback/{id}"), View),
        ("PATCH", format!("/feedback/{id}"), Act),
        ("GET", format!("/feedback/{id}/attachments/{PK}"), View),
        ("GET", "/operators".into(), Operator),
        ("PUT", format!("/operators/{PK}"), Operator),
        ("DELETE", format!("/operators/{PK}"), Operator),
        ("GET", "/communities".into(), View),
        ("GET", format!("/members/search?{host}&q=a"), View),
        ("GET", format!("/members/{PK}?{host}"), View),
        ("GET", format!("/events/{PK}?{host}"), View),
        ("GET", format!("/members/restrictions?{host}"), View),
        ("DELETE", format!("/members/{PK}/ban?{host}"), Act),
        ("DELETE", format!("/members/{PK}/timeout?{host}"), Act),
        ("POST", format!("/members/{PK}/ban?{host}"), Act),
        ("POST", format!("/members/{PK}/timeout?{host}"), Act),
        ("POST", format!("/events/{PK}/delete?{host}"), Act),
    ]
}

/// Counts registrations, not paths: it catches an added or removed `.route(`
/// call, not a changed path within one.
#[test]
fn every_route_is_listed() {
    let source = include_str!("mod.rs");
    let start = source.find("pub fn router(").expect("router() in mod.rs");
    let body = &source[start..];
    let body = &body[..body.find("\n}\n").expect("end of router()")];
    assert_eq!(body.matches(".route(").count(), every_route().len());
}

fn unsigned_request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "admin.example")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap()
}

/// nip98 demands a credential on every route; disabled mode serves every view
/// route and refuses every act and operator route.
///
/// The route list is written by hand: a new route must be added here on
/// purpose. View rows only prove no credential is asked for (they accept 404
/// or 500); `community_reads_serve_disabled_mode_within_the_selected_community`
/// is what proves the reads return data. Unsigned and disabled-mode requests
/// cannot tell `.operator()` from `.act()`; `moderator_cannot_access_staffing_endpoints`
/// covers that boundary.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn every_route_makes_its_check_in_both_auth_modes() {
    let signed = nip98_state(vec![test_operator_keys().public_key().to_hex()]).await;
    let open = disabled_mode_state().await;
    for (method, uri, check) in every_route() {
        let (status, _) = send(&signed, unsigned_request(method, &uri)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "nip98 {method} {uri}");

        let (status, body) = send(&open, unsigned_request(method, &uri)).await;
        if check == Check::View {
            assert!(
                status != StatusCode::UNAUTHORIZED && status != StatusCode::FORBIDDEN,
                "disabled {method} {uri} ({check:?}) answered {status}: {body}"
            );
        } else {
            assert_eq!(
                (status, body["error"]["message"].as_str()),
                (
                    StatusCode::FORBIDDEN,
                    Some("this endpoint requires BUZZ_ADMIN_AUTH=nip98")
                ),
                "disabled {method} {uri} ({check:?})"
            );
        }
    }
}
