//! Unit tests for the relay HTTP/command bridge helpers.
//! Extracted from `relay.rs` to keep that module under the file-size ratchet.

use super::{
    build_profile_event, classify_intercepted_response, effective_agent_relay_url,
    extract_retry_in_hint, parse_command_response, relay_http_base_url, MALFORMED_RESPONSE_MESSAGE,
};
use serde::Deserialize;

// ── extract_retry_in_hint ────────────────────────────────────────────────

#[test]
fn extracts_hint_from_429_body() {
    assert_eq!(
        extract_retry_in_hint(r#"{"error":"rate-limited: quota exceeded; retry in 4s"}"#),
        Some(4)
    );
}

#[test]
fn extracts_hint_when_no_json_wrapper() {
    assert_eq!(extract_retry_in_hint("retry in 30s"), Some(30));
}

#[test]
fn returns_none_when_no_hint_present() {
    assert_eq!(
        extract_retry_in_hint(r#"{"error":"rate-limited: quota exceeded"}"#),
        None
    );
    assert_eq!(extract_retry_in_hint(""), None);
}

#[test]
fn overlong_digit_string_returns_none() {
    // A digit sequence that exceeds u64::MAX cannot be parsed; the function
    // must return None (→ caller uses the default) rather than panicking.
    assert_eq!(
        extract_retry_in_hint("retry in 99999999999999999999999s"),
        None
    );
}

// ── relay_error_message: hint capping ────────────────────────────────────
//
// Verify that an oversized relay hint is capped in the returned message
// string, not just inside `activate_rate_limit()`. This guarantees every
// consumer — including the TS gate via `applyTauriRateLimitIfNeeded` —
// receives the capped value rather than the raw untrusted relay value.

#[tokio::test]
async fn oversized_hint_is_capped_in_relay_error_message_string() {
    use crate::relay_admission::{reset_rate_limit_gate, MAX_HINT_SECONDS, TEST_SERIAL};
    use std::io::{Read as _, Write as _};

    let _serial = TEST_SERIAL.lock().await;
    reset_rate_limit_gate();

    // Use a std::net listener on a std::thread — the same pattern as the
    // relay_admission loopback tests. This avoids two races that cause CI
    // failures with tokio::net + into_std():
    //  1. No request read: the client is still sending when the response
    //     arrives → hyper `UnexpectedMessage`/`Canceled` under load.
    //  2. into_std() leaves the socket in nonblocking mode → write_all
    //     may return WouldBlock and silently drop the response.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    // Serve a 429 with a hint far exceeding MAX_HINT_SECONDS (300).
    let oversized = 1_000_000u64;
    let body = format!(r#"{{"error":"rate-limited: quota exceeded; retry in {oversized}s"}}"#);
    let body_len = body.len();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            // Read the request first so the client finishes sending before
            // we write the response — mirrors relay_admission.rs pattern.
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n{body}"
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{addr}/"))
        .send()
        .await
        .expect("request must succeed");

    let msg = super::relay_error_message(response).await;

    // The message must embed the CAPPED hint, not the raw 1 000 000.
    assert_eq!(
        msg,
        format!("relay rate-limited: retry in {MAX_HINT_SECONDS}s"),
        "relay_error_message must embed the capped hint, not the raw untrusted value"
    );
    assert!(
        !msg.contains(&oversized.to_string()),
        "raw oversized hint must not appear in the message string"
    );
    reset_rate_limit_gate();
}

// ── effective_agent_relay_url: legacy pin ignored ─────────────────────────

#[test]
fn stored_relay_pin_is_ignored() {
    // Zero-touch cutover (#2122): a creation-era per-record relay pin is
    // parsed and persisted but never consulted — the workspace relay wins.
    assert_eq!(
        effective_agent_relay_url("wss://relay.other.com", "wss://staging.example.com"),
        "wss://staging.example.com"
    );
}

#[test]
fn empty_relay_resolves_to_workspace() {
    // A never-set record resolves to the active workspace relay at read-time,
    // so a stale stored default can never make it load-bearing.
    assert_eq!(
        effective_agent_relay_url("", "wss://staging.example.com"),
        "wss://staging.example.com"
    );
}

#[test]
fn whitespace_only_relay_resolves_to_workspace() {
    // Whitespace-only behaves identically — no value survives.
    assert_eq!(
        effective_agent_relay_url("   ", "wss://staging.example.com"),
        "wss://staging.example.com"
    );
}

// ── relay_http_base_url scheme conversion ────────────────────────────────

#[test]
fn loopback_ws_localhost_preserves_authority() {
    // Tenant host-binding keys off the HTTP Host/authority. The desktop must
    // not rewrite localhost to 127.0.0.1, or local dev HTTP calls target a
    // different unmapped community than the WebSocket URL.
    assert_eq!(
        relay_http_base_url("ws://localhost:3000"),
        "http://localhost:3000"
    );
}

#[test]
fn loopback_trailing_slash_removed_authority_preserved() {
    assert_eq!(
        relay_http_base_url("ws://localhost:3000/"),
        "http://localhost:3000"
    );
}

#[test]
fn remote_wss_host_unchanged() {
    assert_eq!(
        relay_http_base_url("wss://relay.example.com"),
        "https://relay.example.com"
    );
}

#[test]
fn loopback_ipv4_literal_unchanged() {
    assert_eq!(
        relay_http_base_url("ws://127.0.0.1:3000"),
        "http://127.0.0.1:3000"
    );
}

#[test]
fn localhost_substring_host_unchanged() {
    assert_eq!(
        relay_http_base_url("ws://localhost.evil.com:3000"),
        "http://localhost.evil.com:3000"
    );
}

#[test]
fn loopback_wss_localhost_preserves_authority() {
    assert_eq!(
        relay_http_base_url("wss://localhost:3000"),
        "https://localhost:3000"
    );
}

// ── classify_intercepted_response ────────────────────────────────────────

#[test]
fn intercepted_cloudflare_host_returns_some() {
    let result = classify_intercepted_response("sqprod.cloudflareaccess.com", "text/html");
    assert!(result.is_some());
    let msg = result.unwrap();
    assert!(
        msg.starts_with("relay unreachable:"),
        "should have unreachable prefix"
    );
    assert!(msg.contains("Cloudflare"), "should mention Cloudflare");
}

#[test]
fn intercepted_cloudflare_apex_host_returns_some() {
    // The apex domain itself should also match.
    let result = classify_intercepted_response("cloudflareaccess.com", "application/json");
    assert!(result.is_some());
    let msg = result.unwrap();
    assert!(msg.starts_with("relay unreachable:"));
    assert!(msg.contains("Cloudflare"));
}

#[test]
fn intercepted_non_cloudflare_html_returns_some() {
    let result =
        classify_intercepted_response("proxy.corporate.example", "text/html; charset=utf-8");
    assert!(result.is_some());
    let msg = result.unwrap();
    assert!(msg.starts_with("relay unreachable:"));
}

#[test]
fn normal_relay_json_returns_none() {
    let result = classify_intercepted_response("relay.myapp.example.com", "application/json");
    assert!(result.is_none());
}

#[test]
fn content_type_case_insensitive() {
    // Uppercase content-type must still be detected.
    let result = classify_intercepted_response("proxy.example.com", "TEXT/HTML");
    assert!(result.is_some());
    assert!(result.unwrap().starts_with("relay unreachable:"));
}

#[test]
fn evil_suffix_does_not_match_cloudflare() {
    // A host whose suffix happens to contain the Cloudflare string but is
    // not actually a subdomain must NOT match.
    let result =
        classify_intercepted_response("notcloudflareaccess.com.evil.example", "application/json");
    assert!(
        result.is_none(),
        "false suffix match should not trigger Cloudflare branch"
    );
}

// classify_request_error requires a real reqwest::Error (not publicly
// constructable) — tested indirectly through integration; skipped here.

// ── /query per-request timeout → classified error ────────────────────────
//
// A stalled `/query` connection (headers never arrive) must not hang the
// caller forever. Both production `/query` builders funnel through
// `send_query_request`, which owns the per-request `.timeout(...)`; this test
// drives that exact helper against a loopback server that accepts the
// connection but never responds. It asserts two things the frontend depends
// on: (1) the helper returns instead of hanging, and (2) the failure is the
// stable `"relay unreachable: request timed out"` classified string.
//
// The outer `tokio::time::timeout` is the regression guard: if the production
// `.timeout(...)` is ever removed from `send_query_request`, this call would
// hang forever, so the guard fires and the test fails fast rather than
// stalling CI. A short 200ms deadline keeps the happy path fast.
#[tokio::test]
async fn stalled_query_request_times_out_with_classified_error() {
    use std::io::Read as _;
    use std::time::Duration;

    // A listener that accepts the connection and then holds it open without
    // ever writing a response — the "headers never arrive" stall.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            // Drain the request but deliberately never respond, then hold
            // the socket until the client aborts on its own timeout.
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            std::thread::sleep(Duration::from_secs(2));
        }
    });

    let client = reqwest::Client::new();
    let url = format!("http://{addr}/query");
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        super::send_query_request(
            &client,
            &url,
            "Nostr test-auth",
            None,
            b"[]".to_vec(),
            Duration::from_millis(200),
        ),
    )
    .await
    .expect(
        "send_query_request must honor its per-request timeout and resolve within 5s; \
         if this guard fires, the production .timeout(...) was lost",
    );

    let err = result.expect_err("a stalled /query must surface an error, not succeed");
    assert_eq!(
        err, "relay unreachable: request timed out",
        "a timed-out /query must surface the stable classified string"
    );

    let _ = handle.join();
}

// ── /query body-stall timeout → classified error (not malformed) ─────────
//
// `send()` resolves once response headers arrive, so a relay that returns a
// valid 2xx JSON header block and then stalls the body trips the request
// deadline inside `response.json()` — the branch the pre-header stall above
// cannot reach. That is a connectivity failure, not a malformed body, so it
// must surface the stable "relay unreachable: request timed out" string rather
// than the malformed-response bucket. This drives `send_query_request` against
// a loopback that writes headers promising a body it never sends.
#[tokio::test]
async fn stalled_response_body_times_out_with_classified_error() {
    use std::io::{Read as _, Write as _};
    use std::time::Duration;

    // Accept, drain the request, write a complete 2xx JSON header block that
    // promises a body (Content-Length), then send nothing and hold the socket
    // — the "headers arrive, body stalls" half-open case.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 64\r\n\r\n",
            );
            let _ = stream.flush();
            // Never write the promised body; hold past the client deadline.
            std::thread::sleep(Duration::from_secs(2));
        }
    });

    let client = reqwest::Client::new();
    let url = format!("http://{addr}/query");
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        super::send_query_request(
            &client,
            &url,
            "Nostr test-auth",
            None,
            b"[]".to_vec(),
            Duration::from_millis(200),
        ),
    )
    .await
    .expect(
        "send_query_request must honor its per-request timeout through body \
         consumption and resolve within 5s",
    );

    let err = result.expect_err("a stalled response body must surface an error, not succeed");
    assert_eq!(
        err, "relay unreachable: request timed out",
        "a body-stall timeout must surface the classified timeout string, not the \
         malformed-response bucket"
    );

    let _ = handle.join();
}

// ── /query non-2xx body-stall timeout → classified error (not status) ────
//
// The 2xx path is not the only body-consuming path. A relay that returns a
// non-success status (500, 429, …) routes through `relay_error_message`, which
// consumes the body via `text()` to extract the structured error field. If the
// relay sends the status headers and then stalls the promised body, that
// consumption trips the same request deadline — and it must surface the stable
// "relay unreachable: request timed out" classification, not a bare
// "relay returned 500" that hides the connectivity failure. This drives
// `send_query_request` against a loopback that writes 500 headers promising a
// body it never sends.
#[tokio::test]
async fn stalled_error_response_body_times_out_with_classified_error() {
    use std::io::{Read as _, Write as _};
    use std::time::Duration;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            // 500 status headers promising a body (Content-Length) that never
            // arrives — the "error headers arrive, body stalls" half-open case.
            let _ = stream.write_all(
                b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: 64\r\n\r\n",
            );
            let _ = stream.flush();
            std::thread::sleep(Duration::from_secs(2));
        }
    });

    let client = reqwest::Client::new();
    let url = format!("http://{addr}/query");
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        super::send_query_request(
            &client,
            &url,
            "Nostr test-auth",
            None,
            b"[]".to_vec(),
            Duration::from_millis(200),
        ),
    )
    .await
    .expect(
        "send_query_request must honor its per-request timeout through error-body \
         consumption and resolve within 5s",
    );

    let err = result.expect_err("a stalled error-response body must surface an error, not succeed");
    assert_eq!(
        err, "relay unreachable: request timed out",
        "a non-2xx body-stall timeout must surface the classified timeout string, not the \
         status bucket"
    );

    let _ = handle.join();
}

// ── /query non-stalled 500 → status message (timeout preservation is scoped) ─
//
// The timeout preservation above must not swallow genuine relay errors: a 500
// whose body arrives promptly still surfaces as "relay returned 500". This
// pins that `classify_body_timeout` only fires on an actual timeout, so the
// error-classification path stays intact for live relay failures.
#[tokio::test]
async fn non_stalled_error_response_yields_status_message() {
    use std::io::{Read as _, Write as _};
    use std::time::Duration;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            // A complete 500 with a non-JSON body delivered immediately.
            let body = "internal error";
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    let client = reqwest::Client::new();
    let url = format!("http://{addr}/query");
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        super::send_query_request(
            &client,
            &url,
            "Nostr test-auth",
            None,
            b"[]".to_vec(),
            Duration::from_millis(200),
        ),
    )
    .await
    .expect("a promptly-served 500 must resolve well within 5s");

    let err = result.expect_err("a 500 must surface an error, not succeed");
    assert_eq!(
        err, "relay returned 500 Internal Server Error",
        "a non-stalled 500 must keep its status classification, not be reclassified as a timeout"
    );

    let _ = handle.join();
}

// ── parse_json_response malformed-body contract ──────────────────────────

#[test]
fn malformed_response_message_stays_off_unreachable_bucket() {
    // A reached-but-malformed 2xx body is not a connectivity failure. If this
    // message ever regains the "relay unreachable:" prefix, the frontend
    // classifier would misroute it as unreachable — pin that it never does.
    assert!(
        !MALFORMED_RESPONSE_MESSAGE.starts_with("relay unreachable:"),
        "malformed-response message must not match the unreachable prefix"
    );
}

// ── parse_command_response ───────────────────────────────────────────────

#[derive(Debug, Deserialize, PartialEq)]
struct ChannelCreated {
    channel_id: String,
}

#[test]
fn parse_command_response_decodes_typed_payload() {
    let msg = r#"response:{"channel_id":"abc123"}"#;
    let parsed: ChannelCreated = parse_command_response(msg).expect("should parse");
    assert_eq!(
        parsed,
        ChannelCreated {
            channel_id: "abc123".to_string()
        }
    );
}

#[test]
fn parse_command_response_accepts_raw_json_fallback() {
    // Backward-compat: relays that emit raw JSON (no prefix) still work.
    let msg = r#"{"channel_id":"abc"}"#;
    let parsed: ChannelCreated = parse_command_response(msg).expect("fallback parse");
    assert_eq!(
        parsed,
        ChannelCreated {
            channel_id: "abc".to_string()
        }
    );
}

#[test]
fn parse_command_response_rejects_invalid_prefixed_json() {
    let msg = "response:not-json";
    let result: Result<ChannelCreated, _> = parse_command_response(msg);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("response parse failed"));
}

#[test]
fn parse_command_response_rejects_garbage() {
    let msg = "totally not json or response";
    let result: Result<ChannelCreated, _> = parse_command_response(msg);
    assert!(result.is_err());
}

// ── build_profile_event ──────────────────────────────────────────────────

/// Generate a valid NIP-OA auth tag JSON string signed by a fresh owner key
/// and addressed to `agent_keys`.
///
/// Uses `nostr_compat` (nostr 0.36) for the owner keys because
/// `buzz_sdk_pkg::nip_oa::compute_auth_tag` expects nostr 0.36 types.
/// The agent pubkey is bridged via hex encoding.
fn make_valid_auth_tag(agent_keys: &nostr::Keys) -> String {
    let owner_keys = nostr::Keys::generate();
    let agent_pubkey_hex = agent_keys.public_key().to_hex();
    let agent_compat_pubkey =
        nostr::PublicKey::from_hex(&agent_pubkey_hex).expect("valid hex pubkey should parse");
    buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner_keys, &agent_compat_pubkey, "")
        .expect("compute_auth_tag should not fail with distinct keys")
}

#[test]
fn profile_event_with_valid_auth_tag() {
    let agent_keys = nostr::Keys::generate();
    let tag_json = make_valid_auth_tag(&agent_keys);
    let event = build_profile_event(&agent_keys, "TestBot", None, None, Some(&tag_json))
        .expect("should succeed with a valid auth tag");

    // Exactly one "auth" tag must be present.
    let auth_tags: Vec<_> = event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("auth"))
        .collect();
    assert_eq!(auth_tags.len(), 1, "expected exactly 1 auth tag");

    // Must be a kind:0 (Metadata) event.
    assert_eq!(event.kind, nostr::Kind::Metadata);
}

#[test]
fn profile_event_without_auth_tag() {
    let agent_keys = nostr::Keys::generate();
    let event = build_profile_event(&agent_keys, "TestBot", None, None, None)
        .expect("should succeed without an auth tag");

    // No "auth" tags should be present.
    let auth_tags: Vec<_> = event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("auth"))
        .collect();
    assert_eq!(auth_tags.len(), 0, "expected no auth tags");

    assert_eq!(event.kind, nostr::Kind::Metadata);
}

#[test]
fn profile_event_includes_about_when_description_present() {
    let agent_keys = nostr::Keys::generate();
    let event = build_profile_event(
        &agent_keys,
        "TestBot",
        None,
        Some("A meticulous code reviewer."),
        None,
    )
    .expect("should succeed with an about");
    let content: serde_json::Value =
        serde_json::from_str(&event.content).expect("kind:0 content is JSON");
    assert_eq!(
        content.get("about").and_then(|v| v.as_str()),
        Some("A meticulous code reviewer.")
    );
}

#[test]
fn profile_event_omits_about_when_absent() {
    let agent_keys = nostr::Keys::generate();
    let event = build_profile_event(&agent_keys, "TestBot", None, None, None)
        .expect("should succeed without an about");
    let content: serde_json::Value =
        serde_json::from_str(&event.content).expect("kind:0 content is JSON");
    assert!(content.get("about").is_none());
}

#[test]
fn profile_event_rejects_invalid_auth_tag() {
    let agent_keys = nostr::Keys::generate();
    // Structurally valid JSON array but with a bogus signature — verification must fail.
    let bad_json = format!(r#"["auth","{}","","{}"]"#, "a".repeat(64), "b".repeat(128));
    let result = build_profile_event(&agent_keys, "TestBot", None, None, Some(&bad_json));
    assert!(result.is_err(), "should reject an invalid auth tag");
    assert!(
        result.unwrap_err().contains("verification failed"),
        "error message should mention verification failure"
    );
}
