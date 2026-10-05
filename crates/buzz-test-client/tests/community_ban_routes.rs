//! Community-ban coverage for the relay's registered HTTP and root WebSocket routes.
//!
//! The route inventory is shared with `scripts/check-community-ban-route-inventory.py`.
//! This ignored test uses a disposable local relay/PostgreSQL/MinIO stack and
//! fresh tenant hosts and identities. GIF requests stop at malformed local
//! input before any provider request; Git and workflow requests use absent
//! resources and must stop before subprocess or durable work.
//!
//! ```text
//! ./scripts/start-relay-for-tests.sh
//! RELAY_URL=ws://localhost:3000 RELAY_HTTP_URL=http://localhost:3000 \
//!   cargo test -p buzz-test-client --test community_ban_routes -- --ignored --nocapture
//! ```

use std::{collections::BTreeSet, time::Duration};

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use buzz_test_client::{BuzzTestClient, TestClientError};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use reqwest::{Client, Method, Response, StatusCode};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Postgres};
use uuid::Uuid;

const INVENTORY: &str = include_str!("fixtures/community-ban-route-inventory.tsv");
const BANNED_MESSAGE: &str = "blocked: you are banned from this community";

const COMMUNITY_BAN_MATRIX_CASES: &[&str] = &[
    "events",
    "moderation_command",
    "query",
    "count",
    "gif_search",
    "gif_share",
    "workflow_runs",
    "workflow_approvals",
    "invite_mint",
    "moderation_reports",
    "moderation_owner_ban",
    "moderation_audit",
    "moderation_restricted",
    "media_upload",
    "media_get",
    "media_head",
    "git_info_refs",
    "git_upload_pack",
    "git_receive_pack",
    "git_default_get",
    "git_default_post",
];

#[derive(Clone, Debug)]
struct RouteRow {
    id: String,
    method: String,
    path: String,
    exercise: String,
    control: String,
}

#[derive(Clone)]
struct Principal {
    name: &'static str,
    keys: Keys,
    nip_oa_tag: Option<String>,
}

fn relay_http_url() -> String {
    std::env::var("RELAY_HTTP_URL").unwrap_or_else(|_| {
        relay_url()
            .replace("wss://", "https://")
            .replace("ws://", "http://")
    })
}

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn relay_authority() -> String {
    let url = url::Url::parse(&relay_http_url()).expect("relay HTTP URL");
    url[url::Position::BeforeHost..url::Position::AfterPort].to_string()
}

fn http_origin_for_host(host: &str) -> String {
    let scheme = if relay_http_url().starts_with("https://") {
        "https"
    } else {
        "http"
    };
    format!("{scheme}://{host}")
}

fn test_host() -> String {
    format!(
        "community-ban-{}.{}",
        Uuid::new_v4().simple(),
        relay_authority()
    )
}

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .or_else(|_| std::env::var("BUZZ_TEST_DATABASE_URL"))
        .expect("set DATABASE_URL or BUZZ_TEST_DATABASE_URL to a disposable relay-test database")
}

async fn pool() -> Pool<Postgres> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url())
        .await
        .expect("connect to disposable relay-test PostgreSQL")
}

async fn ensure_community(pool: &Pool<Postgres>, host: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO communities (id, host) VALUES ($1, $2) \
         ON CONFLICT (lower(host)) DO NOTHING",
    )
    .bind(id)
    .bind(host)
    .execute(pool)
    .await
    .unwrap_or_else(|error| panic!("seed disposable community {host}: {error}"));
    sqlx::query_scalar("SELECT id FROM communities WHERE lower(host) = lower($1)")
        .bind(host)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|error| panic!("lookup disposable community {host}: {error}"))
}

async fn seed_member(pool: &Pool<Postgres>, community: Uuid, keys: &Keys, role: &str) {
    sqlx::query("INSERT INTO users (community_id, pubkey) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(community)
        .bind(keys.public_key().to_bytes().to_vec())
        .execute(pool)
        .await
        .unwrap_or_else(|error| panic!("seed {role} user: {error}"));
    sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, $3, NULL) \
         ON CONFLICT (community_id, pubkey) DO UPDATE \
         SET role = EXCLUDED.role, updated_at = now()",
    )
    .bind(community)
    .bind(keys.public_key().to_hex())
    .bind(role)
    .execute(pool)
    .await
    .unwrap_or_else(|error| panic!("seed {role} relay member: {error}"));
}

async fn ban_member(pool: &Pool<Postgres>, community: Uuid, actor: &Keys, target: &Keys) {
    sqlx::query(
        "INSERT INTO community_bans (community_id, pubkey, banned, actor_pubkey) \
         VALUES ($1, $2, TRUE, $3) \
         ON CONFLICT (community_id, pubkey) DO UPDATE \
         SET banned = TRUE, actor_pubkey = EXCLUDED.actor_pubkey",
    )
    .bind(community)
    .bind(target.public_key().to_bytes().to_vec())
    .bind(actor.public_key().to_bytes().to_vec())
    .execute(pool)
    .await
    .unwrap_or_else(|error| panic!("seed community ban: {error}"));
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn nip98_header(keys: &Keys, url: &str, method: &str, body: &[u8]) -> String {
    let mut tags = vec![
        Tag::parse(["u", url]).expect("NIP-98 URL tag"),
        Tag::parse(["method", method]).expect("NIP-98 method tag"),
        Tag::parse(["nonce", &Uuid::new_v4().to_string()]).expect("NIP-98 nonce tag"),
    ];
    if !body.is_empty() {
        tags.push(Tag::parse(["payload", &sha256_hex(body)]).expect("NIP-98 payload tag"));
    }
    let event = EventBuilder::new(Kind::Custom(27_235), "")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign NIP-98 event");
    format!("Nostr {}", STANDARD.encode(event.as_json().as_bytes()))
}

fn blossom_header(keys: &Keys, tag: &str, hash: &str, host: &str) -> String {
    let expiration = (Timestamp::now().as_secs() + 55).to_string();
    let tags = vec![
        Tag::parse(["t", tag]).expect("Blossom operation tag"),
        Tag::parse(["x", hash]).expect("Blossom hash tag"),
        Tag::parse(["expiration", &expiration]).expect("Blossom expiration tag"),
        Tag::parse(["server", host]).expect("Blossom server tag"),
    ];
    let event = EventBuilder::new(Kind::Custom(24_242), "community-ban route test")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign Blossom auth event");
    format!(
        "Nostr {}",
        URL_SAFE_NO_PAD.encode(event.as_json().as_bytes())
    )
}

fn parse_inventory() -> Vec<RouteRow> {
    INVENTORY
        .lines()
        .skip(1)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let columns: Vec<_> = line.split('\t').collect();
            assert_eq!(
                columns.len(),
                9,
                "malformed community-ban inventory row: {line}"
            );
            RouteRow {
                id: columns[0].to_string(),
                method: columns[2].to_string(),
                path: columns[3].to_string(),
                exercise: columns[7].to_string(),
                control: columns[8].to_string(),
            }
        })
        .collect()
}

fn route_path(row: &RouteRow, case: &str, actor: &Principal) -> String {
    let uuid = Uuid::new_v4();
    let repo_owner = actor.keys.public_key().to_hex();
    let missing_repo = format!("community-ban-{}", uuid.simple());
    let missing_blob = sha256_hex(format!("missing-{}", uuid.simple()).as_bytes());
    let mut path = row
        .path
        .replace("{workflow_id}", &uuid.to_string())
        .replace("{run_id}", &Uuid::new_v4().to_string())
        .replace("{sha256_ext}", &format!("{missing_blob}.jpg"))
        .replace("{owner}", &repo_owner)
        .replace("{repo}", &missing_repo);
    if case == "git_info_refs" {
        path.push_str("?service=git-upload-pack");
    }
    path
}

fn request_body(case: &str, actor: &Principal) -> Vec<u8> {
    match case {
        "events" => EventBuilder::new(Kind::TextNote, "community-ban route matrix")
            .sign_with_keys(&actor.keys)
            .expect("sign test message")
            .as_json()
            .into_bytes(),
        "moderation_command" => {
            let target = Keys::generate();
            EventBuilder::new(Kind::Custom(9_040), "")
                .tags([Tag::parse(["p", &target.public_key().to_hex()]).expect("target tag")])
                .sign_with_keys(&actor.keys)
                .expect("sign ban command")
                .as_json()
                .into_bytes()
        }
        "query" | "count" => serde_json::to_vec(&serde_json::json!([{
            "kinds": [1],
            "limit": 1
        }]))
        .expect("serialize Nostr filter"),
        "gif_search" | "gif_share" | "invite_mint" => b"{".to_vec(),
        "media_upload" => b"not an image file".to_vec(),
        "git_upload_pack" | "git_receive_pack" => Vec::new(),
        "git_default_post" => serde_json::to_vec(&serde_json::json!({
            "branch": "main",
            "expected_manifest": "missing"
        }))
        .expect("serialize default-branch request"),
        _ => Vec::new(),
    }
}

fn git_signing_method<'a>(case: &str, request_method: &'a str) -> &'a str {
    match case {
        // Smart-HTTP clone and push intentionally reuse a GET-scoped NIP-98
        // credential for their POST pack requests.
        "git_upload_pack" | "git_receive_pack" => "GET",
        _ => request_method,
    }
}

fn nip98_url_for_case(origin: &str, path: &str, case: &str) -> String {
    let repo_root = match case {
        "git_info_refs" => path.split_once("/info/refs").map(|(prefix, _)| prefix),
        "git_upload_pack" => path.strip_suffix("/git-upload-pack"),
        "git_receive_pack" => path.strip_suffix("/git-receive-pack"),
        _ => None,
    };
    match repo_root {
        Some(repo_root) => format!("{origin}{repo_root}"),
        None => format!("{origin}{path}"),
    }
}

async fn send_route_request(
    client: &Client,
    row: &RouteRow,
    case: &str,
    actor: &Principal,
    host: &str,
) -> Response {
    let path = route_path(row, case, actor);
    let url = format!("{}{}", relay_http_url().trim_end_matches('/'), path);
    let signed_url = nip98_url_for_case(&http_origin_for_host(host), &path, case);
    let body = request_body(case, actor);
    let method = Method::from_bytes(row.method.as_bytes()).expect("inventory HTTP method");
    let mut request = client
        .request(method.clone(), url)
        .header(reqwest::header::HOST, host)
        .header(
            reqwest::header::CONTENT_TYPE,
            if case == "media_upload" {
                "image/jpeg"
            } else {
                "application/json"
            },
        );

    if case == "media_upload" || case == "media_get" || case == "media_head" {
        let (operation, hash) = if case == "media_upload" {
            ("upload", sha256_hex(&body))
        } else {
            (
                "get",
                path.split('/')
                    .next_back()
                    .unwrap_or_default()
                    .split('.')
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            )
        };
        let auth = blossom_header(&actor.keys, operation, &hash, host);
        request = request.header(reqwest::header::AUTHORIZATION, auth);
        if case == "media_upload" {
            request = request.header("X-SHA-256", hash);
        }
    } else {
        let auth_method = git_signing_method(case, row.method.as_str());
        let auth = nip98_header(&actor.keys, &signed_url, auth_method, &body);
        request = request.header(reqwest::header::AUTHORIZATION, auth);
    }

    if let Some(tag) = &actor.nip_oa_tag {
        request = request.header("x-auth-tag", tag);
    }
    if !body.is_empty()
        || method == Method::POST
        || method == Method::PUT
        || method == Method::PATCH
    {
        request = request.body(body);
    }
    request
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .unwrap_or_else(|error| {
            panic!("{} {} for {} failed: {error}", row.method, path, actor.name)
        })
}

fn matrix_cases(row: &RouteRow) -> Vec<&str> {
    row.exercise
        .strip_prefix("matrix:")
        .expect("route-matrix exercise")
        .split('+')
        .collect()
}

fn assert_ban_denial(
    response_status: StatusCode,
    body: &str,
    route: &RouteRow,
    case: &str,
    actor: &str,
) {
    assert_eq!(
        response_status,
        StatusCode::FORBIDDEN,
        "{} {} case {case}, {actor} must be denied by the ban gate; body: {body}",
        route.method,
        route.path
    );
    if route.id.starts_with("git_default_branch") {
        assert_eq!(
            body, r#"{"error":"blocked: you are banned from this community"}"#,
            "{} {} must preserve Git settings' ban denial response; body: {body}",
            route.method, route.path
        );
    } else if route.id.starts_with("git_") {
        assert!(
            body.contains("blocked: banned from this community"),
            "{} {} must preserve Git's ban denial contract; body: {body}",
            route.method,
            route.path
        );
    } else if route.method == "HEAD" {
        assert!(
            body.is_empty(),
            "{} {} must suppress the denial body for HEAD; body: {body}",
            route.method,
            route.path
        );
    } else if route.id.starts_with("media_") {
        // Blossom deliberately maps both non-membership and ban refusals to
        // the same policy response. These fixtures are direct relay members,
        // and the paired clear member control proves this is the ban gate.
        assert_eq!(
            body, r#"{"error":"relay membership required"}"#,
            "{} {} must preserve Blossom's policy-denial response; body: {body}",
            route.method, route.path
        );
    } else {
        assert!(
            body.contains(BANNED_MESSAGE),
            "{} {} must preserve the community ban denial contract; body: {body}",
            route.method,
            route.path
        );
    }
}

fn assert_allowed_control(response_status: StatusCode, body: &str, route: &RouteRow, case: &str) {
    assert!(
        matches!(response_status.as_u16(), 200..=299 | 400 | 404 | 409 | 415 | 422),
        "{} {} case {case}: unbanned control must pass authentication and authorization, then reach the fixture's expected success or input/resource result; got {}: {body}",
        route.method,
        route.path,
        response_status
    );
    let denial_message = if route.id.starts_with("git_default_branch") {
        BANNED_MESSAGE
    } else if route.id.starts_with("git_") {
        "blocked: banned from this community"
    } else if route.id.starts_with("media_") {
        "relay membership required"
    } else {
        BANNED_MESSAGE
    };
    assert!(
        !body.contains(denial_message),
        "{} {} case {case}: allowed control must not receive a ban denial; body: {body}",
        route.method,
        route.path
    );
}

fn principal<'a>(name: &str, principals: &[&'a Principal]) -> &'a Principal {
    principals
        .iter()
        .find(|principal| principal.name == name)
        .unwrap_or_else(|| panic!("unknown route-matrix control principal {name}"))
}

#[tokio::test]
#[ignore]
async fn community_http_ban_matrix() {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("build HTTP client");
    let db = pool().await;
    let host = test_host();
    let community = ensure_community(&db, &host).await;
    let implemented_cases: BTreeSet<_> = COMMUNITY_BAN_MATRIX_CASES.iter().copied().collect();
    let inventory = parse_inventory();
    let inventory_cases: BTreeSet<_> = inventory
        .iter()
        .filter(|route| route.exercise.starts_with("matrix:"))
        .flat_map(matrix_cases)
        .collect();
    assert_eq!(
        inventory_cases, implemented_cases,
        "every route-matrix exercise must have a behavior implementation"
    );

    let banned_member = Principal {
        name: "member",
        keys: Keys::generate(),
        nip_oa_tag: None,
    };
    let banned_admin = Principal {
        name: "admin",
        keys: Keys::generate(),
        nip_oa_tag: None,
    };
    let banned_agent_keys = Keys::generate();
    let banned_agent = Principal {
        name: "agent",
        nip_oa_tag: Some(
            buzz_sdk::nip_oa::compute_auth_tag(
                &banned_admin.keys,
                &banned_agent_keys.public_key(),
                "",
            )
            .expect("sign delegated-agent owner proof"),
        ),
        keys: banned_agent_keys,
    };
    let banned_moderator_agent_keys = Keys::generate();
    let banned_moderator_agent = Principal {
        name: "moderator-agent",
        nip_oa_tag: Some(
            buzz_sdk::nip_oa::compute_auth_tag(
                &banned_admin.keys,
                &banned_moderator_agent_keys.public_key(),
                "",
            )
            .expect("sign moderator-agent owner proof"),
        ),
        keys: banned_moderator_agent_keys,
    };
    let clear_member = Principal {
        name: "member",
        keys: Keys::generate(),
        nip_oa_tag: None,
    };
    let clear_admin = Principal {
        name: "admin",
        keys: Keys::generate(),
        nip_oa_tag: None,
    };
    let clear_owner = Principal {
        name: "owner",
        keys: Keys::generate(),
        nip_oa_tag: None,
    };
    let clear_moderator_agent_keys = Keys::generate();
    let clear_moderator_agent = Principal {
        name: "clear-moderator-agent",
        nip_oa_tag: Some(
            buzz_sdk::nip_oa::compute_auth_tag(
                &clear_owner.keys,
                &clear_moderator_agent_keys.public_key(),
                "",
            )
            .expect("sign clear moderator-agent owner proof"),
        ),
        keys: clear_moderator_agent_keys,
    };
    seed_member(&db, community, &banned_member.keys, "member").await;
    seed_member(&db, community, &banned_admin.keys, "admin").await;
    seed_member(&db, community, &clear_member.keys, "member").await;
    seed_member(&db, community, &clear_admin.keys, "admin").await;
    seed_member(&db, community, &clear_owner.keys, "owner").await;
    seed_member(&db, community, &banned_moderator_agent.keys, "admin").await;
    seed_member(&db, community, &clear_moderator_agent.keys, "admin").await;
    ban_member(&db, community, &clear_owner.keys, &banned_member.keys).await;
    ban_member(&db, community, &clear_owner.keys, &banned_admin.keys).await;

    let banned = [&banned_member, &banned_admin, &banned_agent];
    let controls = [
        &clear_member,
        &clear_admin,
        &clear_owner,
        &clear_moderator_agent,
    ];
    for route in parse_inventory()
        .into_iter()
        .filter(|route| route.exercise.starts_with("matrix:"))
    {
        for case in matrix_cases(&route) {
            let ban_actors: Vec<&Principal> = if case == "moderation_owner_ban" {
                vec![&banned_moderator_agent]
            } else {
                banned.to_vec()
            };
            for actor in ban_actors {
                let response = send_route_request(&client, &route, case, actor, &host).await;
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                assert_ban_denial(status, &body, &route, case, actor.name);
            }

            let control_name = match case {
                "moderation_command" => "owner",
                "moderation_owner_ban" => "clear-moderator-agent",
                _ => route.control.as_str(),
            };
            let control = principal(control_name, &controls);
            let response = send_route_request(&client, &route, case, control, &host).await;
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            assert_allowed_control(status, &body, &route, case);
        }
    }
}

#[tokio::test]
#[ignore]
async fn ban_is_scoped_to_host_resolved_community() {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("build HTTP client");
    let db = pool().await;
    let host_a = test_host();
    let host_b = test_host();
    let community_a = ensure_community(&db, &host_a).await;
    let community_b = ensure_community(&db, &host_b).await;
    let actor = Principal {
        name: "tenant-control",
        keys: Keys::generate(),
        nip_oa_tag: None,
    };
    let issuer = Keys::generate();
    seed_member(&db, community_a, &actor.keys, "member").await;
    seed_member(&db, community_b, &actor.keys, "member").await;
    ban_member(&db, community_a, &issuer, &actor.keys).await;

    let route = parse_inventory()
        .into_iter()
        .find(|route| route.id == "http_query")
        .expect("query route is inventoried");
    let in_b = send_route_request(&client, &route, "query", &actor, &host_b).await;
    let in_b_status = in_b.status();
    let in_b_body = in_b.text().await.unwrap_or_default();
    assert_eq!(
        in_b_status,
        StatusCode::OK,
        "ban in A must not block B: {in_b_body}"
    );

    let in_a = send_route_request(&client, &route, "query", &actor, &host_a).await;
    let in_a_status = in_a.status();
    let in_a_body = in_a.text().await.unwrap_or_default();
    assert_ban_denial(in_a_status, &in_a_body, &route, "query", actor.name);
}

#[tokio::test]
#[ignore]
async fn banned_root_websocket_is_refused() {
    let db = pool().await;
    let host = relay_authority();
    let community = ensure_community(&db, &host).await;
    let issuer = Keys::generate();
    let banned = Keys::generate();
    let clear = Keys::generate();
    let banned_owner = Keys::generate();
    let owner_agent = Keys::generate();
    ban_member(&db, community, &issuer, &banned).await;
    ban_member(&db, community, &issuer, &banned_owner).await;

    match BuzzTestClient::connect(&relay_url(), &banned).await {
        Err(error) => assert!(
            error.to_string().to_ascii_lowercase().contains("banned"),
            "root WebSocket rejection must identify the ban, got {error}"
        ),
        Ok(client) => {
            let _ = client.disconnect().await;
            panic!("a community-banned principal authenticated on the root WebSocket")
        }
    }

    let auth_tag = buzz_sdk::nip_oa::compute_auth_tag(&banned_owner, &owner_agent.public_key(), "")
        .expect("sign owner proof for WebSocket agent");
    let auth_tag: Vec<String> = serde_json::from_str(&auth_tag).expect("decode owner proof");
    let auth_tag = Tag::parse(auth_tag).expect("parse NIP-OA auth tag");
    let mut agent_socket = BuzzTestClient::connect_unauthenticated(&relay_url())
        .await
        .expect("open root WebSocket for owner-banned agent");
    let error = agent_socket
        .authenticate_with_nip_oa(&owner_agent, &auth_tag)
        .await
        .expect_err("a NIP-OA agent of a banned owner must be refused");
    assert!(
        error.to_string().to_ascii_lowercase().contains("banned"),
        "root WebSocket owner ban must identify the restriction, got {error}"
    );
    drop(agent_socket);

    let control = BuzzTestClient::connect(&relay_url(), &clear)
        .await
        .unwrap_or_else(|error: TestClientError| {
            panic!("unbanned root WebSocket control must authenticate: {error}")
        });
    control
        .disconnect()
        .await
        .expect("disconnect unbanned root WebSocket control");
}
