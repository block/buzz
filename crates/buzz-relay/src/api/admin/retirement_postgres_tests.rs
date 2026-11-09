//! Route journey for identity retirement: Operator-only, feature-gated,
//! inventory-bound archive with NIP-IA publication, replay and conflict,
//! visibility-only unarchive, and release.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use buzz_core::CommunityId;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

use super::super::auth::ADMIN_API_PREFIX;
use super::super::postgres_tests::{
    make_nostr_auth, make_nostr_auth_raw_tags, nip98_state_with_real_pool, test_operator_keys,
};
use super::super::router;
use crate::state::AppState;
use crate::test_support::database_url;

const ROOT: [u8; 32] = [0xa1; 32];
const AGENT: [u8; 32] = [0xa2; 32];

async fn send(state: &Arc<AppState>, request: Request<Body>) -> (StatusCode, Value) {
    let response = router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn call(
    state: &Arc<AppState>,
    keys: &nostr::Keys,
    method: &str,
    uri: &str,
    body: &Value,
) -> (StatusCode, Value) {
    let body = serde_json::to_vec(body).unwrap();
    let url = format!("https://admin.example{ADMIN_API_PREFIX}{uri}");
    let auth = make_nostr_auth_raw_tags(
        keys,
        vec![
            nostr::Tag::parse(["u", &url]).unwrap(),
            nostr::Tag::parse(["method", method]).unwrap(),
            nostr::Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
        ],
    );
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "admin.example")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, auth)
        .body(Body::from(body))
        .unwrap();
    send(state, request).await
}

async fn state(enabled: bool) -> (sqlx::PgPool, Arc<AppState>) {
    let pool = sqlx::PgPool::connect(&database_url()).await.unwrap();
    let mut state = nip98_state_with_real_pool(pool.clone()).await;
    let state_mut = Arc::get_mut(&mut state).expect("fresh state");
    Arc::make_mut(&mut state_mut.config).admin_identity_retirement = enabled;
    (pool, state)
}

async fn community(pool: &sqlx::PgPool) -> (CommunityId, String) {
    let host = format!("retire-{}.example", Uuid::new_v4().simple());
    let c = buzz_db::Db::from_pool(pool.clone())
        .ensure_configured_community(&host)
        .await
        .unwrap()
        .id;
    for (key, owner) in [(ROOT, None), (AGENT, Some(ROOT))] {
        sqlx::query(
            "INSERT INTO users (community_id, pubkey, agent_owner_pubkey) VALUES ($1, $2, $3)",
        )
        .bind(c.as_uuid())
        .bind(key.as_slice())
        .bind(owner.as_ref().map(|o| o.as_slice()))
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO workflows (community_id, owner_pubkey, name, definition, definition_hash) \
         VALUES ($1, $2, 'example', '{}', $2)",
    )
    .bind(c.as_uuid())
    .bind(AGENT.as_slice())
    .execute(pool)
    .await
    .unwrap();
    (c, host)
}

async fn tags(pool: &sqlx::PgPool, c: CommunityId, kind: i32) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT tags FROM events WHERE community_id = $1 AND kind = $2 ORDER BY created_at, id",
    )
    .bind(c.as_uuid())
    .bind(kind)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn retirement_routes_are_operator_only_gated_bound_and_idempotent() {
    let operator = test_operator_keys();
    let root = hex::encode(ROOT);
    let agent = hex::encode(AGENT);

    // Disabled: an Operator learns it is disabled; nothing is written.
    let (pool, off) = state(false).await;
    let (c, host) = community(&pool).await;
    let archive = format!("/members/{root}/archive?communityHost={host}");
    let body = json!({"requestId": Uuid::new_v4(), "inventoryVersion": "0".repeat(64)});
    let (status, err) = call(&off, &operator, "POST", &archive, &body).await;
    assert_eq!(
        (status, err["error"]["code"].as_str()),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Some("identity_retirement_disabled")
        )
    );

    let (_, on) = state(true).await;
    // A moderator is refused before anything else.
    let moderator = nostr::Keys::generate();
    on.db
        .upsert_relay_operator(
            &moderator.public_key().to_bytes(),
            "moderator",
            &[1; 32],
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        call(&on, &moderator, "POST", &archive, &body).await.0,
        StatusCode::FORBIDDEN
    );

    // Validation.
    for (bad, code) in [
        (json!({"requestId": Uuid::new_v4()}), "invalid_body"),
        (
            json!({"requestId": Uuid::new_v4(), "inventoryVersion": "0".repeat(64), "extra": 1}),
            "invalid_body",
        ),
        (
            json!({"requestId": Uuid::new_v4(), "inventoryVersion": "0".repeat(64), "reason": "Left Co"}),
            "invalid_reason",
        ),
        (
            json!({"requestId": Uuid::new_v4(), "inventoryVersion": "0".repeat(64), "replacedBy": root}),
            "invalid_replaced_by",
        ),
    ] {
        let (status, err) = call(&on, &operator, "POST", &archive, &bad).await;
        assert_eq!(
            (status, err["error"]["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some(code)),
            "{bad}"
        );
    }
    let upper = format!(
        "/members/{}/archive?communityHost={host}",
        root.to_uppercase()
    );
    assert_eq!(
        call(&on, &operator, "POST", &upper, &body).await.1["error"]["code"],
        "invalid_pubkey"
    );
    let staff = format!(
        "/members/{}/archive?communityHost={host}",
        operator.public_key().to_hex()
    );
    assert_eq!(
        call(&on, &operator, "POST", &staff, &body).await.1["error"]["code"],
        "target_is_staff"
    );
    assert_eq!(
        call(&on, &operator, "POST", &archive, &body).await.1["error"]["code"],
        "inventory_changed"
    );

    // Archive against the confirmed inventory.
    let ownership = format!("/members/{root}/ownership?communityHost={host}");
    let request = Request::builder()
        .uri(&ownership)
        .header(header::HOST, "admin.example")
        .header(
            header::AUTHORIZATION,
            make_nostr_auth(&operator, &ownership),
        )
        .body(Body::empty())
        .unwrap();
    let (status, inventory) = send(&on, request).await;
    assert_eq!(status, StatusCode::OK);
    let version = inventory["inventoryVersion"].as_str().unwrap().to_owned();
    let request_id = Uuid::new_v4();
    let body =
        json!({"requestId": request_id, "reason": "left-company", "inventoryVersion": version});
    let (status, first) = call(&on, &operator, "POST", &archive, &body).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["replayed"], false);
    assert_eq!(first["communityHost"], host);
    assert_eq!(first["pubkey"], root);
    assert_eq!(first["identities"].as_array().unwrap().len(), 2);
    assert_eq!(first["workflows"][0]["status"], "archived");
    assert_eq!(
        first["stages"],
        json!({
            "visibility": "confirmed",
            "newWorkAdmission": "blocked",
            "workflows": "confirmed",
            "inFlightRuns": "none",
            "agentRuntime": "unsupported",
        })
    );

    // NIP-IA: relay-consent deltas without request reference, and the snapshot.
    let deltas = tags(&pool, c, 8002).await;
    assert_eq!(deltas.len(), 2);
    for delta in &deltas {
        let delta = delta.as_array().unwrap();
        assert!(delta.contains(&json!(["consent", "relay"])), "{delta:?}");
        assert!(!delta.iter().any(|t| t[0] == "e"), "{delta:?}");
    }
    let snapshot = tags(&pool, c, 13535).await;
    let latest = snapshot.last().unwrap().as_array().unwrap();
    assert!(latest.contains(&json!(["p", root])) && latest.contains(&json!(["p", agent])));

    // Replay: same stored result, no new deltas. Any change: conflict.
    let (status, again) = call(&on, &operator, "POST", &archive, &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["replayed"], true);
    assert_eq!(again["actionId"], first["actionId"]);
    assert_eq!(again["identities"], first["identities"]);
    assert_eq!(tags(&pool, c, 8002).await.len(), 2);
    let changed = json!({"requestId": request_id, "reason": "other", "inventoryVersion": version});
    let (status, err) = call(&on, &operator, "POST", &archive, &changed).await;
    assert_eq!(
        (status, err["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("request_id_conflict"))
    );

    // Unarchive restores visibility only.
    let (status, unarchived) = call(
        &on,
        &operator,
        "DELETE",
        &archive,
        &json!({"requestId": Uuid::new_v4()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{unarchived}");
    assert_eq!(unarchived["identities"][0]["archived"], false);
    assert_eq!(unarchived["identities"][0]["executionHeld"], true);
    assert_eq!(unarchived["stages"]["newWorkAdmission"], "unchanged");
    let undeltas = tags(&pool, c, 8003).await;
    assert_eq!(undeltas.len(), 1);
    assert!(undeltas[0]
        .as_array()
        .unwrap()
        .contains(&json!(["p", root])));

    // Release the agent, then the root.
    let hold = |key: &str| format!("/members/{key}/execution-hold?communityHost={host}");
    let (status, released) = call(
        &on,
        &operator,
        "DELETE",
        &hold(&agent),
        &json!({"requestId": Uuid::new_v4()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{released}");
    assert_eq!(released["stages"]["visibility"], "unchanged");
    assert_eq!(released["stages"]["newWorkAdmission"], "blocked");
    assert_eq!(released["workflows"][0]["status"], "active");
    // Inspected directly, the released agent still shows its owner's hold.
    let agent_view = format!("/members/{agent}/ownership?communityHost={host}");
    let request = Request::builder()
        .uri(&agent_view)
        .header(header::HOST, "admin.example")
        .header(
            header::AUTHORIZATION,
            make_nostr_auth(&operator, &agent_view),
        )
        .body(Body::empty())
        .unwrap();
    let (_, view) = send(&on, request).await;
    assert_eq!(view["identities"][0]["executionHeld"], false, "{view}");
    assert_eq!(view["identities"][0]["ownerExecutionHeld"], true, "{view}");
    let (_, released) = call(
        &on,
        &operator,
        "DELETE",
        &hold(&root),
        &json!({"requestId": Uuid::new_v4()}),
    )
    .await;
    assert_eq!(released["stages"]["newWorkAdmission"], "released");
}
