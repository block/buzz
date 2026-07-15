/**
 * Unit tests for parseInviteInput — the strict invite URL/code parser
 * added in Phase 2 (D3). Covers all three plan-specced input forms,
 * HTTPS→WSS / HTTP→WS canonicalization, and rejection cases.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { parseInviteInput } from "./inviteHelpers.ts";

// ---------------------------------------------------------------------------
// HTTPS invite URLs → canonical wss:// relay
// ---------------------------------------------------------------------------

test("parseInviteInput_https_invite_url_returns_wss_relay_and_code", () => {
  const result = parseInviteInput("https://relay.example.com/invite/abc123");
  assert.deepEqual(result, {
    relayWsUrl: "wss://relay.example.com",
    code: "abc123",
  });
});

test("parseInviteInput_http_invite_url_returns_ws_relay_and_code", () => {
  const result = parseInviteInput("http://relay.example.com/invite/abc123");
  assert.deepEqual(result, {
    relayWsUrl: "ws://relay.example.com",
    code: "abc123",
  });
});

test("parseInviteInput_https_with_port_preserves_port", () => {
  const result = parseInviteInput(
    "https://relay.example.com:8443/invite/abc123",
  );
  assert.deepEqual(result, {
    relayWsUrl: "wss://relay.example.com:8443",
    code: "abc123",
  });
});

test("parseInviteInput_https_trailing_slash_tolerated", () => {
  const result = parseInviteInput("https://relay.example.com/invite/abc123/");
  assert.deepEqual(result, {
    relayWsUrl: "wss://relay.example.com",
    code: "abc123",
  });
});

test("parseInviteInput_https_url_encoded_code_decoded", () => {
  const result = parseInviteInput(
    "https://relay.example.com/invite/hello%20world",
  );
  assert.deepEqual(result, {
    relayWsUrl: "wss://relay.example.com",
    code: "hello world",
  });
});

// ---------------------------------------------------------------------------
// buzz://join deep link URLs
// ---------------------------------------------------------------------------

test("parseInviteInput_buzz_join_with_wss_relay_returns_relay_and_code", () => {
  const result = parseInviteInput(
    "buzz://join?relay=wss://relay.example.com&code=abc123",
  );
  assert.deepEqual(result, {
    relayWsUrl: "wss://relay.example.com",
    code: "abc123",
  });
});

test("parseInviteInput_buzz_join_with_ws_relay_returns_relay_and_code", () => {
  const result = parseInviteInput(
    "buzz://join?relay=ws://localhost:3000&code=testcode",
  );
  assert.deepEqual(result, {
    relayWsUrl: "ws://localhost:3000",
    code: "testcode",
  });
});

test("parseInviteInput_buzz_join_with_encoded_relay_param", () => {
  const result = parseInviteInput(
    "buzz://join?relay=wss%3A%2F%2Frelay.example.com&code=abc123",
  );
  assert.deepEqual(result, {
    relayWsUrl: "wss://relay.example.com",
    code: "abc123",
  });
});

test("parseInviteInput_buzz_join_rejects_non_ws_relay", () => {
  const result = parseInviteInput(
    "buzz://join?relay=https://relay.example.com&code=abc123",
  );
  assert.equal(result, null);
});

test("parseInviteInput_buzz_join_rejects_missing_code", () => {
  assert.equal(
    parseInviteInput("buzz://join?relay=wss://relay.example.com"),
    null,
  );
});

test("parseInviteInput_buzz_join_rejects_missing_relay", () => {
  assert.equal(parseInviteInput("buzz://join?code=abc123"), null);
});

// ---------------------------------------------------------------------------
// Bare codes
// ---------------------------------------------------------------------------

test("parseInviteInput_bare_code_returns_code_only", () => {
  const result = parseInviteInput("abc123");
  assert.deepEqual(result, { code: "abc123" });
});

test("parseInviteInput_bare_code_trims_whitespace", () => {
  const result = parseInviteInput("  abc123  ");
  assert.deepEqual(result, { code: "abc123" });
});

// ---------------------------------------------------------------------------
// Rejection cases
// ---------------------------------------------------------------------------

test("parseInviteInput_rejects_empty_input", () => {
  assert.equal(parseInviteInput(""), null);
  assert.equal(parseInviteInput("   "), null);
});

test("parseInviteInput_rejects_credentials_in_https_url", () => {
  assert.equal(
    parseInviteInput("https://user:pass@relay.example.com/invite/abc123"),
    null,
  );
});

test("parseInviteInput_rejects_fragment_in_https_url", () => {
  assert.equal(
    parseInviteInput("https://relay.example.com/invite/abc123#section"),
    null,
  );
});

test("parseInviteInput_rejects_non_invite_https_pathname", () => {
  // /api/invites is not /invite/<code>
  assert.equal(
    parseInviteInput("https://relay.example.com/api/invites/abc123"),
    null,
  );
  // Root path
  assert.equal(parseInviteInput("https://relay.example.com/"), null);
  // Missing code segment
  assert.equal(parseInviteInput("https://relay.example.com/invite/"), null);
});

test("parseInviteInput_rejects_buzz_join_with_credentials", () => {
  assert.equal(
    parseInviteInput(
      "buzz://user:pass@join?relay=wss://relay.example.com&code=abc123",
    ),
    null,
  );
});

test("parseInviteInput_rejects_buzz_join_with_hash", () => {
  assert.equal(
    parseInviteInput(
      "buzz://join?relay=wss://relay.example.com&code=abc123#frag",
    ),
    null,
  );
});

test("parseInviteInput_rejects_ws_wss_scheme_urls", () => {
  // ws/wss URLs are relay URLs, not invite URLs
  assert.equal(parseInviteInput("wss://relay.example.com"), null);
  assert.equal(parseInviteInput("ws://relay.example.com"), null);
});

test("parseInviteInput_rejects_input_with_slashes_as_bare_code", () => {
  // Slashes suggest a URL but don't parse as a valid invite URL
  assert.equal(parseInviteInput("not/a/code"), null);
});

test("parseInviteInput_rejects_input_with_scheme_as_bare_code", () => {
  assert.equal(parseInviteInput("ftp://something"), null);
});

// ---------------------------------------------------------------------------
// Nested relay param credential/fragment rejection (MINOR 6 regression)
// ---------------------------------------------------------------------------

test("parseInviteInput_buzz_join_rejects_credentials_in_nested_relay", () => {
  assert.equal(
    parseInviteInput(
      "buzz://join?relay=wss%3A%2F%2Fuser%3Apass%40relay.example.com&code=abc",
    ),
    null,
  );
});

test("parseInviteInput_buzz_join_rejects_fragment_in_nested_relay", () => {
  assert.equal(
    parseInviteInput(
      "buzz://join?relay=wss%3A%2F%2Frelay.example.com%23frag&code=abc",
    ),
    null,
  );
});
