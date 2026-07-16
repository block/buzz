use super::{
    build_profile_event, classify_intercepted_response, effective_agent_relay_url,
    normalize_relay_url, parse_command_response, relay_http_base_url, relay_urls_equivalent,
    MALFORMED_RESPONSE_MESSAGE,
};
use serde::Deserialize;

// ── effective_agent_relay_url: per-agent override precedence ─────────────

#[test]
fn explicit_relay_wins_over_workspace() {
    // An explicit per-agent relay pins the agent there regardless of the
    // active workspace — this is the override taking highest precedence.
    assert_eq!(
        effective_agent_relay_url("wss://relay.other.com", "wss://staging.example.com"),
        "wss://relay.other.com"
    );
}

#[test]
fn explicit_relay_wins_even_when_equal_to_workspace() {
    // No special-casing when the pin happens to match the active workspace.
    assert_eq!(
        effective_agent_relay_url("wss://staging.example.com", "wss://staging.example.com"),
        "wss://staging.example.com"
    );
}

#[test]
fn empty_relay_falls_back_to_workspace() {
    // A never-set record resolves to the active workspace relay at read-time,
    // so a stale stored default can never make it load-bearing.
    assert_eq!(
        effective_agent_relay_url("", "wss://staging.example.com"),
        "wss://staging.example.com"
    );
}

#[test]
fn whitespace_only_relay_falls_back_to_workspace() {
    // Whitespace-only is treated as unset, same as empty.
    assert_eq!(
        effective_agent_relay_url("   ", "wss://staging.example.com"),
        "wss://staging.example.com"
    );
}

// ── normalize_relay_url / relay_urls_equivalent ──────────────────────────

/// One vector of the shared fixture consumed by BOTH this test and the
/// frontend mirror's (`agentRelayScope.test.mjs`): record pins are
/// stamped by `normalize_relay_url` and compared by the frontend's
/// `normalizeRelayUrlForCompare`, so an edit that lands on only one side
/// must fail the other side's tests instead of shipping a scoping skew.
#[derive(Deserialize)]
struct NormalizationVector {
    input: String,
    canonical: String,
}

#[test]
fn normalize_agrees_with_shared_frontend_fixture() {
    let vectors: Vec<NormalizationVector> = serde_json::from_str(include_str!(
        "../../../fixtures/relay-url-normalization.json"
    ))
    .unwrap();
    assert!(!vectors.is_empty(), "fixture must not be empty");
    for vector in &vectors {
        assert_eq!(
            normalize_relay_url(&vector.input),
            vector.canonical,
            "input: {:?}",
            vector.input
        );
    }
}

#[test]
fn normalize_strips_whitespace_and_trailing_slashes() {
    assert_eq!(
        normalize_relay_url("  wss://relay.example.com//  "),
        "wss://relay.example.com"
    );
}

#[test]
fn normalize_lowercases_scheme_and_host_only() {
    // Scheme and authority are case-insensitive (RFC 3986); a path is not.
    assert_eq!(
        normalize_relay_url("WSS://Relay.Example.COM:3000/Path"),
        "wss://relay.example.com:3000/Path"
    );
}

#[test]
fn normalize_passes_through_schemeless_values() {
    // Not a URL — nothing to case-fold beyond trim/slash cleanup.
    assert_eq!(normalize_relay_url("not-a-url/"), "not-a-url");
}

#[test]
fn equivalence_ignores_cosmetic_differences() {
    assert!(relay_urls_equivalent(
        "wss://Relay.Example/",
        " wss://relay.example"
    ));
}

#[test]
fn equivalence_distinguishes_real_differences() {
    // Different host, port, or scheme = a different relay.
    assert!(!relay_urls_equivalent(
        "wss://relay-a.example",
        "wss://relay-b.example"
    ));
    assert!(!relay_urls_equivalent(
        "ws://relay.example:3000",
        "ws://relay.example:3001"
    ));
    assert!(!relay_urls_equivalent(
        "ws://relay.example",
        "wss://relay.example"
    ));
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
    let event = build_profile_event(&agent_keys, "TestBot", None, Some(&tag_json))
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
    let event = build_profile_event(&agent_keys, "TestBot", None, None)
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
fn profile_event_rejects_invalid_auth_tag() {
    let agent_keys = nostr::Keys::generate();
    // Structurally valid JSON array but with a bogus signature — verification must fail.
    let bad_json = format!(r#"["auth","{}","","{}"]"#, "a".repeat(64), "b".repeat(128));
    let result = build_profile_event(&agent_keys, "TestBot", None, Some(&bad_json));
    assert!(result.is_err(), "should reject an invalid auth tag");
    assert!(
        result.unwrap_err().contains("verification failed"),
        "error message should mention verification failure"
    );
}
