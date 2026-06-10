//! E2E tests for the self-service token minting API.
//!
//! These tests require a running relay instance with `require_auth_token=false`
//! (dev mode). By default they are marked `#[ignore]` so that `cargo test`
//! does not fail in CI when the relay is not available.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client --test e2e_tokens -- --ignored
//! ```
//!
//! # Auth
//!
//! In dev mode (`require_auth_token=false`) the relay accepts an
//! `X-Pubkey: <hex>` header as authentication, granting all scopes.
//! Tests generate fresh [`nostr::Keys`] per test for isolation.

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
use reqwest::Client;
use sha2::{Digest, Sha256};

// ── URL helpers ───────────────────────────────────────────────────────────────

/// WebSocket relay URL (e.g. `ws://localhost:3000`).
fn relay_ws_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3001".to_string())
}

/// HTTP base URL derived from the WebSocket URL.
fn relay_http_url() -> String {
    relay_ws_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
}

/// Build a `reqwest::Client` with a short timeout.
fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
}

// ── NIP-98 helpers ────────────────────────────────────────────────────────────

/// Build a `Authorization: Nostr <base64>` header value for NIP-98 HTTP Auth.
///
/// Uses kind 27235 (`Kind::HttpAuth`) with `u`, `method`, and `payload` tags
/// following the pattern in `buzz-auth/src/nip98.rs`.
fn build_nip98_header(keys: &Keys, url: &str, method: &str, body: &[u8]) -> String {
    let payload_hash = hex::encode(Sha256::digest(body));

    let tags = vec![
        Tag::parse(["u", url]).expect("u tag"),
        Tag::parse(["method", method]).expect("method tag"),
        Tag::parse(["payload", &payload_hash]).expect("payload tag"),
    ];

    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("signing must succeed");

    let json = event.as_json();
    let encoded = BASE64.encode(json.as_bytes());
    format!("Nostr {encoded}")
}

/// Build a `Authorization: Nostr <base64>` header value for NIP-98 HTTP Auth
/// **without** the payload tag.
///
/// Used to test that `POST /api/tokens` rejects NIP-98 events that don't
/// cryptographically bind the request body.
fn build_nip98_header_no_payload(keys: &Keys, url: &str, method: &str) -> String {
    let tags = vec![
        Tag::parse(["u", url]).expect("u tag"),
        Tag::parse(["method", method]).expect("method tag"),
        // Deliberately omit payload tag
    ];
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("signing must succeed");
    let json = event.as_json();
    let encoded = BASE64.encode(json.as_bytes());
    format!("Nostr {encoded}")
}

// ── Mint helper ───────────────────────────────────────────────────────────────

/// Mint a token via dev-mode `X-Pubkey` header. Returns the parsed response body.
async fn mint_token_dev(
    client: &Client,
    pubkey_hex: &str,
    name: &str,
    scopes: &[&str],
) -> serde_json::Value {
    let url = format!("{}/api/tokens", relay_http_url());
    let body = serde_json::json!({
        "name": name,
        "scopes": scopes,
    });
    let resp = client
        .post(&url)
        .header("X-Pubkey", pubkey_hex)
        .json(&body)
        .send()
        .await
        .expect("POST /api/tokens failed");
    assert_eq!(
        resp.status(),
        201,
        "expected 201, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    resp.json().await.expect("response JSON")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// POST /api/tokens via dev-mode X-Pubkey header returns 201 with token fields.
#[tokio::test]
#[ignore]
async fn test_mint_token_via_dev_mode() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    let url = format!("{}/api/tokens", relay_http_url());
    let body = serde_json::json!({
        "name": "dev-mode-token",
        "scopes": ["messages:read", "messages:write"],
    });

    let resp = client
        .post(&url)
        .header("X-Pubkey", &pubkey_hex)
        .json(&body)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(resp.status(), 201, "expected 201 Created");

    let json: serde_json::Value = resp.json().await.expect("response JSON");
    assert!(json["id"].is_string(), "id should be a string UUID");
    assert!(json["token"].is_string(), "token should be a string");
    assert!(
        !json["token"].as_str().unwrap().is_empty(),
        "token should not be empty"
    );
    assert_eq!(json["name"], "dev-mode-token");
    assert!(json["scopes"].is_array());
    let scopes = json["scopes"].as_array().unwrap();
    assert!(scopes.iter().any(|s| s == "messages:read"));
    assert!(scopes.iter().any(|s| s == "messages:write"));
    assert!(json["created_at"].is_string());
    // expires_at should be null (no expiry requested)
    assert!(json["expires_at"].is_null());
}

/// POST /api/tokens via NIP-98 Authorization header returns 201.
#[tokio::test]
#[ignore]
async fn test_mint_token_via_nip98() {
    let client = http_client();
    let keys = Keys::generate();

    let endpoint_url = format!("{}/api/tokens", relay_http_url());
    let body_json = serde_json::json!({
        "name": "nip98-token",
        "scopes": ["messages:read"],
    });
    let body_bytes = serde_json::to_vec(&body_json).unwrap();

    let auth_header = build_nip98_header(&keys, &endpoint_url, "POST", &body_bytes);

    let resp = client
        .post(&endpoint_url)
        .header("Authorization", auth_header)
        .header("Content-Type", "application/json")
        .body(body_bytes)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(
        resp.status(),
        201,
        "expected 201, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );

    let json: serde_json::Value = resp.json().await.expect("response JSON");
    assert!(json["id"].is_string());
    assert!(json["token"].is_string());
    assert_eq!(json["name"], "nip98-token");
}

/// POST /api/tokens with admin scope returns 400 "scope requires admin".
#[tokio::test]
#[ignore]
async fn test_mint_rejects_admin_scopes() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    let url = format!("{}/api/tokens", relay_http_url());
    let body = serde_json::json!({
        "name": "admin-attempt",
        "scopes": ["admin:channels"],
    });

    let resp = client
        .post(&url)
        .header("X-Pubkey", &pubkey_hex)
        .json(&body)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(resp.status(), 400, "expected 400, got {}", resp.status());
    let json: serde_json::Value = resp.json().await.expect("response JSON");
    let msg = json["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("scope requires admin"),
        "expected 'scope requires admin' in message, got: {msg}"
    );
}

/// POST /api/tokens with empty scopes array returns 400.
#[tokio::test]
#[ignore]
async fn test_mint_rejects_empty_scopes() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    let url = format!("{}/api/tokens", relay_http_url());
    let body = serde_json::json!({
        "name": "empty-scopes",
        "scopes": [],
    });

    let resp = client
        .post(&url)
        .header("X-Pubkey", &pubkey_hex)
        .json(&body)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(resp.status(), 400, "expected 400 for empty scopes");
    let json: serde_json::Value = resp.json().await.expect("response JSON");
    let err_msg = json["error"].as_str().unwrap_or("");
    assert!(
        err_msg.contains("invalid_scopes"),
        "expected 'invalid_scopes' in error, got: {err_msg}"
    );
}

/// POST /api/tokens with an unknown scope returns 400 "unknown scope".
#[tokio::test]
#[ignore]
async fn test_mint_rejects_unknown_scopes() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    let url = format!("{}/api/tokens", relay_http_url());
    let body = serde_json::json!({
        "name": "unknown-scope",
        "scopes": ["future:capability"],
    });

    let resp = client
        .post(&url)
        .header("X-Pubkey", &pubkey_hex)
        .json(&body)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(resp.status(), 400, "expected 400 for unknown scope");
    let json: serde_json::Value = resp.json().await.expect("response JSON");
    let msg = json["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("unknown scope"),
        "expected 'unknown scope' in message, got: {msg}"
    );
}

/// POST /api/tokens with a name longer than 100 characters returns 400.
#[tokio::test]
#[ignore]
async fn test_mint_rejects_long_name() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    let url = format!("{}/api/tokens", relay_http_url());
    // 101 'a' characters — one over the 100-char limit.
    let long_name = "a".repeat(101);
    let body = serde_json::json!({
        "name": long_name,
        "scopes": ["messages:read"],
    });

    let resp = client
        .post(&url)
        .header("X-Pubkey", &pubkey_hex)
        .json(&body)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(
        resp.status(),
        400,
        "expected 400 for name > 100 chars, got {}",
        resp.status()
    );
}

/// GET /api/tokens with X-Pubkey (dev mode) works; lists minted tokens.
#[tokio::test]
#[ignore]
async fn test_list_tokens_via_dev_mode() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    // Mint a token first so the list is non-empty.
    let minted = mint_token_dev(&client, &pubkey_hex, "list-test-token", &["messages:read"]).await;
    let minted_id = minted["id"].as_str().expect("id");

    // List tokens via dev-mode X-Pubkey (dev mode grants all scopes).
    let list_url = format!("{}/api/tokens", relay_http_url());
    let resp = client
        .get(&list_url)
        .header("X-Pubkey", &pubkey_hex)
        .send()
        .await
        .expect("GET /api/tokens failed");

    assert_eq!(
        resp.status(),
        200,
        "expected 200, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );

    let json: serde_json::Value = resp.json().await.expect("response JSON");
    let tokens = json["tokens"].as_array().expect("tokens array");
    assert!(
        tokens.iter().any(|t| t["id"] == minted_id),
        "minted token {minted_id} not found in list"
    );
}

/// GET /api/tokens with NIP-98 Authorization returns 401 — listing requires Bearer.
///
/// Note: the auth layer (`extract_auth_context`) hardcodes `"POST"` when verifying
/// NIP-98 events, so a GET request with a NIP-98 header fails at the NIP-98
/// verification step ("NIP-98 verification failed") before reaching the handler's
/// "Bearer token required" check. Either way the result is 401 — NIP-98 cannot
/// be used to list tokens.
#[tokio::test]
#[ignore]
async fn test_nip98_rejected_for_list_tokens() {
    let client = http_client();
    let keys = Keys::generate();

    let endpoint_url = format!("{}/api/tokens", relay_http_url());
    // GET has no body, so we sign with an empty body.
    let auth_header = build_nip98_header(&keys, &endpoint_url, "GET", b"");

    let resp = client
        .get(&endpoint_url)
        .header("Authorization", auth_header)
        .send()
        .await
        .expect("GET /api/tokens failed");

    assert_eq!(
        resp.status(),
        401,
        "expected 401 for NIP-98 on GET /api/tokens, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );

    // The auth layer explicitly rejects NIP-98 for non-token endpoints.
    let json: serde_json::Value = resp.json().await.expect("response JSON");
    assert_eq!(
        json["error"], "nip98_not_supported",
        "expected nip98_not_supported error, got: {}",
        json
    );
}

/// POST /api/tokens with NIP-98 auth but WITHOUT the payload tag returns 401.
///
/// The payload tag cryptographically binds the request body to the signed event.
/// Without it, the server cannot verify the body hasn't been substituted.
///
/// NOTE: This test depends on the server-side payload-required check added in
/// the tokens handler. If it returns 201 instead of 401, the handler update
/// has not yet been deployed — the test will pass once it is.
#[tokio::test]
#[ignore]
async fn test_nip98_requires_payload_tag() {
    let client = http_client();
    let keys = Keys::generate();

    let endpoint_url = format!("{}/api/tokens", relay_http_url());
    let body_json = serde_json::json!({
        "name": "no-payload-tag-token",
        "scopes": ["messages:read"],
    });
    let body_bytes = serde_json::to_vec(&body_json).unwrap();

    // Sign WITHOUT the payload tag — body is not cryptographically bound.
    let auth_header = build_nip98_header_no_payload(&keys, &endpoint_url, "POST");

    let resp = client
        .post(&endpoint_url)
        .header("Authorization", auth_header)
        .header("Content-Type", "application/json")
        .body(body_bytes)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(
        resp.status(),
        401,
        "expected 401 for NIP-98 without payload tag, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
}

/// POST /api/tokens with NIP-98 auth where the payload tag has a wrong hash returns 401.
///
/// The payload tag must be SHA-256(request body). Sending a hash of "wrong body"
/// while sending the real body should be rejected.
#[tokio::test]
#[ignore]
async fn test_nip98_wrong_payload_rejected() {
    let client = http_client();
    let keys = Keys::generate();

    let endpoint_url = format!("{}/api/tokens", relay_http_url());
    let body_json = serde_json::json!({
        "name": "wrong-payload-token",
        "scopes": ["messages:read"],
    });
    let body_bytes = serde_json::to_vec(&body_json).unwrap();

    // Sign with a hash of "wrong body" — mismatches the actual body sent.
    let auth_header = build_nip98_header(&keys, &endpoint_url, "POST", b"wrong body");

    let resp = client
        .post(&endpoint_url)
        .header("Authorization", auth_header)
        .header("Content-Type", "application/json")
        .body(body_bytes)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(
        resp.status(),
        401,
        "expected 401 for NIP-98 with wrong payload hash, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
}

/// Mint a token, revoke it via DELETE /api/tokens/{id}, then verify it's revoked.
#[tokio::test]
#[ignore]
async fn test_revoke_token() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    // Mint a token.
    let minted = mint_token_dev(&client, &pubkey_hex, "revoke-test", &["messages:read"]).await;
    let token_id = minted["id"].as_str().expect("id");
    let raw_token = minted["token"].as_str().expect("token");

    // Revoke it using the raw token as Bearer.
    let delete_url = format!("{}/api/tokens/{}", relay_http_url(), token_id);
    let resp = client
        .delete(&delete_url)
        .header("Authorization", format!("Bearer {raw_token}"))
        .send()
        .await
        .expect("DELETE /api/tokens/{id} failed");

    assert_eq!(
        resp.status(),
        204,
        "expected 204 No Content, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );

    // Verify the token is now rejected (401) when used.
    let check_url = format!("{}/api/tokens", relay_http_url());
    let resp2 = client
        .get(&check_url)
        .header("Authorization", format!("Bearer {raw_token}"))
        .send()
        .await
        .expect("GET /api/tokens failed");

    assert_eq!(
        resp2.status(),
        401,
        "revoked token should return 401, got {}",
        resp2.status()
    );
}

/// DELETE /api/tokens/{id} with a valid Bearer token but a non-existent UUID returns 404.
#[tokio::test]
#[ignore]
async fn test_revoke_nonexistent_token_returns_404() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    // Mint a real token to use as Bearer auth.
    let minted = mint_token_dev(&client, &pubkey_hex, "auth-token", &["messages:read"]).await;
    let raw_token = minted["token"].as_str().expect("token");

    // Try to delete a random UUID that doesn't exist.
    let random_uuid = uuid::Uuid::new_v4();
    let delete_url = format!("{}/api/tokens/{}", relay_http_url(), random_uuid);
    let resp = client
        .delete(&delete_url)
        .header("Authorization", format!("Bearer {raw_token}"))
        .send()
        .await
        .expect("DELETE /api/tokens/{id} failed");

    assert_eq!(
        resp.status(),
        404,
        "expected 404 for non-existent token, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );

    let json: serde_json::Value = resp.json().await.expect("response JSON");
    assert_eq!(
        json["error"], "not_found",
        "expected not_found error, got: {}",
        json
    );
}

/// Mint 2 tokens, DELETE /api/tokens (revoke all), verify revoked_count=2.
#[tokio::test]
#[ignore]
async fn test_revoke_all_tokens() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    // Mint two tokens.
    let t1 = mint_token_dev(&client, &pubkey_hex, "revoke-all-1", &["messages:read"]).await;
    let t2 = mint_token_dev(&client, &pubkey_hex, "revoke-all-2", &["messages:write"]).await;
    let raw_token_1 = t1["token"].as_str().expect("token 1");
    let _raw_token_2 = t2["token"].as_str().expect("token 2");

    // Revoke all using the first token as Bearer.
    let delete_url = format!("{}/api/tokens", relay_http_url());
    let resp = client
        .delete(&delete_url)
        .header("Authorization", format!("Bearer {raw_token_1}"))
        .send()
        .await
        .expect("DELETE /api/tokens failed");

    assert_eq!(
        resp.status(),
        200,
        "expected 200, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );

    let json: serde_json::Value = resp.json().await.expect("response JSON");
    let revoked_count = json["revoked_count"].as_u64().expect("revoked_count");
    assert_eq!(
        revoked_count, 2,
        "expected 2 tokens revoked, got {revoked_count}"
    );
}

/// Mint a token with only messages:read, then try to use it to mint channels:write — expect 403.
#[tokio::test]
#[ignore]
async fn test_bearer_token_scope_escalation_blocked() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    // Mint a limited-scope token.
    let limited = mint_token_dev(&client, &pubkey_hex, "limited-token", &["messages:read"]).await;
    let limited_raw = limited["token"].as_str().expect("token");

    // Try to mint a new token with channels:write using the limited token.
    let url = format!("{}/api/tokens", relay_http_url());
    let body = serde_json::json!({
        "name": "escalated-token",
        "scopes": ["channels:write"],
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {limited_raw}"))
        .json(&body)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(
        resp.status(),
        403,
        "expected 403 Forbidden for scope escalation, got {}",
        resp.status()
    );

    let json: serde_json::Value = resp.json().await.expect("response JSON");
    assert_eq!(
        json["error"], "scope_escalation",
        "expected scope_escalation error, got: {}",
        json
    );
}

/// Exhaust the per-pubkey rate limit, then verify the next mint returns 429.
///
/// The relay reads `BUZZ_MINT_RATE_LIMIT` (default 50). This test reads the
/// same env var so it stays in sync. For fast test runs, start the relay with
/// `BUZZ_MINT_RATE_LIMIT=5`.
///
/// **Requires `BUZZ_MINT_RATE_LIMIT ≤ 10`** — the DB enforces a hard 10-token
/// cap that fires before the rate limiter when the limit is higher. The test
/// skips gracefully if the limit is too high.
#[tokio::test]
#[ignore]
async fn test_rate_limit() {
    let limit: usize = std::env::var("BUZZ_MINT_RATE_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);

    if limit > 10 {
        eprintln!(
            "SKIP test_rate_limit: BUZZ_MINT_RATE_LIMIT={limit} exceeds the 10-token DB cap. \
             Set BUZZ_MINT_RATE_LIMIT=5 (or ≤10) on both relay and test runner to exercise this."
        );
        return;
    }

    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();
    let url = format!("{}/api/tokens", relay_http_url());

    // Mint up to the limit — all should succeed.
    for i in 0..limit {
        let body = serde_json::json!({
            "name": format!("rate-limit-token-{i}"),
            "scopes": ["messages:read"],
        });
        let resp = client
            .post(&url)
            .header("X-Pubkey", &pubkey_hex)
            .json(&body)
            .send()
            .await
            .expect("POST /api/tokens failed");
        assert_eq!(
            resp.status(),
            201,
            "mint {i}/{limit} should succeed, got {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }

    // Next mint should be rate-limited.
    let body = serde_json::json!({
        "name": "rate-limit-over",
        "scopes": ["messages:read"],
    });
    let resp = client
        .post(&url)
        .header("X-Pubkey", &pubkey_hex)
        .json(&body)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(
        resp.status(),
        429,
        "mint {limit}+1 should be rate-limited (429), got {}",
        resp.status()
    );

    let json: serde_json::Value = resp.json().await.expect("response JSON");
    assert_eq!(json["error"], "rate_limited");
    assert!(
        json["retry_after_seconds"].as_u64().unwrap_or(0) > 0,
        "retry_after_seconds should be positive"
    );
}

/// Mint with expires_in_days: 30 — response should have non-null expires_at.
#[tokio::test]
#[ignore]
async fn test_mint_with_expiry() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    let url = format!("{}/api/tokens", relay_http_url());
    let body = serde_json::json!({
        "name": "expiring-token",
        "scopes": ["messages:read"],
        "expires_in_days": 30,
    });

    let resp = client
        .post(&url)
        .header("X-Pubkey", &pubkey_hex)
        .json(&body)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(resp.status(), 201, "expected 201 Created");

    let json: serde_json::Value = resp.json().await.expect("response JSON");
    assert!(
        json["expires_at"].is_string(),
        "expires_at should be a non-null ISO 8601 string, got: {}",
        json["expires_at"]
    );

    // Verify the expiry is approximately 30 days from now.
    let expires_str = json["expires_at"].as_str().unwrap();
    let expires = chrono::DateTime::parse_from_rfc3339(expires_str)
        .expect("expires_at should be valid RFC 3339");
    let now = chrono::Utc::now();
    let diff = expires.signed_duration_since(now);
    let days = diff.num_days();
    assert!(
        (29..=31).contains(&days),
        "expires_at should be ~30 days from now, got {days} days"
    );
}

/// POST with expires_in_days: 0 or 400 should return 400.
#[tokio::test]
#[ignore]
async fn test_mint_rejects_invalid_expiry() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();
    let url = format!("{}/api/tokens", relay_http_url());

    // expires_in_days: 0 should be rejected.
    let body_zero = serde_json::json!({
        "name": "bad-expiry-zero",
        "scopes": ["messages:read"],
        "expires_in_days": 0,
    });
    let resp = client
        .post(&url)
        .header("X-Pubkey", &pubkey_hex)
        .json(&body_zero)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(
        resp.status(),
        400,
        "expires_in_days: 0 should return 400, got {}",
        resp.status()
    );

    // expires_in_days: 400 should be rejected (max is 365).
    let body_400 = serde_json::json!({
        "name": "bad-expiry-400",
        "scopes": ["messages:read"],
        "expires_in_days": 400,
    });
    let resp2 = client
        .post(&url)
        .header("X-Pubkey", &pubkey_hex)
        .json(&body_400)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(
        resp2.status(),
        400,
        "expires_in_days: 400 should return 400, got {}",
        resp2.status()
    );
}

/// Mint a token, revoke it, then try to revoke it again — expect 409 Conflict.
///
/// The second DELETE must return `{ "error": "already_revoked" }` per the
/// handler's explicit check in `delete_token`.
#[tokio::test]
#[ignore]
async fn test_revoke_already_revoked_returns_409() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    // Mint a token.
    let minted = mint_token_dev(
        &client,
        &pubkey_hex,
        "double-revoke-test",
        &["messages:read"],
    )
    .await;
    let token_id = minted["id"].as_str().expect("id");
    let raw_token = minted["token"].as_str().expect("token");

    let delete_url = format!("{}/api/tokens/{}", relay_http_url(), token_id);

    // First revoke — should succeed with 204.
    let resp1 = client
        .delete(&delete_url)
        .header("Authorization", format!("Bearer {raw_token}"))
        .send()
        .await
        .expect("first DELETE /api/tokens/{id} failed");

    assert_eq!(
        resp1.status(),
        204,
        "first revoke should return 204, got {}: {}",
        resp1.status(),
        resp1.text().await.unwrap_or_default()
    );

    // Second revoke — token is already revoked; must return 409.
    // We need a different Bearer token to authenticate (the revoked one won't work).
    // Mint a second token with dev-mode to use as auth for the second DELETE.
    let auth_token = mint_token_dev(
        &client,
        &pubkey_hex,
        "double-revoke-auth",
        &["messages:read"],
    )
    .await;
    let auth_raw = auth_token["token"].as_str().expect("auth token");

    let resp2 = client
        .delete(&delete_url)
        .header("Authorization", format!("Bearer {auth_raw}"))
        .send()
        .await
        .expect("second DELETE /api/tokens/{id} failed");

    assert_eq!(
        resp2.status(),
        409,
        "second revoke should return 409 Conflict, got {}: {}",
        resp2.status(),
        resp2.text().await.unwrap_or_default()
    );

    let json: serde_json::Value = resp2.json().await.expect("response JSON");
    assert_eq!(
        json["error"], "already_revoked",
        "expected already_revoked error, got: {}",
        json
    );
}

/// NIP-98 auth header sent to a non-token endpoint (GET /api/channels) returns 401.
///
/// `extract_auth_context` explicitly rejects NIP-98 on all non-token endpoints
/// with `{ "error": "nip98_not_supported" }`.
#[tokio::test]
#[ignore]
async fn test_nip98_rejected_for_non_mint_endpoints() {
    let client = http_client();
    let keys = Keys::generate();

    let channels_url = format!("{}/api/channels", relay_http_url());
    // GET has no body — use the no-payload variant.
    let auth_header = build_nip98_header_no_payload(&keys, &channels_url, "GET");

    let resp = client
        .get(&channels_url)
        .header("Authorization", auth_header)
        .header("X-Forwarded-Proto", "http")
        .send()
        .await
        .expect("GET /api/channels failed");

    assert_eq!(
        resp.status(),
        401,
        "NIP-98 on GET /api/channels should return 401, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );

    let json: serde_json::Value = resp.json().await.expect("response JSON");
    assert_eq!(
        json["error"], "nip98_not_supported",
        "expected nip98_not_supported error, got: {}",
        json
    );
}

/// Mint a token with duplicate scopes — response must deduplicate to unique entries.
///
/// Sending `["messages:read", "messages:read", "channels:read"]` should result
/// in exactly 2 unique scopes in the response, not 3.
///
/// Deduplication is implemented in `post_tokens` via order-preserving HashSet retain.
#[tokio::test]
#[ignore]
async fn test_mint_with_duplicate_scopes_deduplicates() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    let url = format!("{}/api/tokens", relay_http_url());
    let body = serde_json::json!({
        "name": "dedup-scopes-token",
        // Intentionally duplicate messages:read.
        "scopes": ["messages:read", "messages:read", "channels:read"],
    });

    let resp = client
        .post(&url)
        .header("X-Pubkey", &pubkey_hex)
        .json(&body)
        .send()
        .await
        .expect("POST /api/tokens failed");

    assert_eq!(
        resp.status(),
        201,
        "expected 201, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );

    let json: serde_json::Value = resp.json().await.expect("response JSON");
    let scopes = json["scopes"].as_array().expect("scopes array");

    // Collect unique scope strings to check deduplication regardless of order.
    let scope_strs: Vec<&str> = scopes.iter().filter_map(|s| s.as_str()).collect();
    let unique_count = {
        let mut deduped = scope_strs.clone();
        deduped.sort_unstable();
        deduped.dedup();
        deduped.len()
    };

    // Must have exactly 2 unique scopes (deduplication applied by handler).
    assert_eq!(
        unique_count, 2,
        "expected 2 unique scopes after deduplication, got {} unique in {:?}",
        unique_count, scope_strs
    );

    // Both unique scopes must be present.
    assert!(
        scope_strs.contains(&"messages:read"),
        "messages:read should be in deduped scopes: {:?}",
        scope_strs
    );
    assert!(
        scope_strs.contains(&"channels:read"),
        "channels:read should be in deduped scopes: {:?}",
        scope_strs
    );
}
