/**
 * Unit tests for relay URL normalization and probe helpers (D4).
 *
 * normalizeRelayUrl is pure and fully testable. probeRelayReachable requires
 * a WebSocket runtime — its contract (cancel-safe, timeout-bounded, close-
 * on-all-exits) is verified by the E2E onboarding specs on CI shards.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { normalizeRelayUrl } from "./relayProbe.ts";

// ---------------------------------------------------------------------------
// Valid inputs
// ---------------------------------------------------------------------------

test("normalizeRelayUrl_wss_passthrough", () => {
  assert.equal(
    normalizeRelayUrl("wss://relay.example.com"),
    "wss://relay.example.com",
  );
});

test("normalizeRelayUrl_ws_passthrough", () => {
  assert.equal(normalizeRelayUrl("ws://localhost:3000"), "ws://localhost:3000");
});

test("normalizeRelayUrl_https_converts_to_wss", () => {
  assert.equal(
    normalizeRelayUrl("https://relay.example.com"),
    "wss://relay.example.com",
  );
});

test("normalizeRelayUrl_http_converts_to_ws", () => {
  assert.equal(
    normalizeRelayUrl("http://localhost:3000"),
    "ws://localhost:3000",
  );
});

test("normalizeRelayUrl_strips_trailing_slashes", () => {
  assert.equal(
    normalizeRelayUrl("wss://relay.example.com///"),
    "wss://relay.example.com",
  );
});

test("normalizeRelayUrl_trims_whitespace", () => {
  assert.equal(
    normalizeRelayUrl("  wss://relay.example.com  "),
    "wss://relay.example.com",
  );
});

test("normalizeRelayUrl_https_with_port_converts_to_wss", () => {
  assert.equal(
    normalizeRelayUrl("https://relay.example.com:8443"),
    "wss://relay.example.com:8443",
  );
});

test("normalizeRelayUrl_wss_with_path_preserves_path", () => {
  assert.equal(
    normalizeRelayUrl("wss://relay.example.com/custom"),
    "wss://relay.example.com/custom",
  );
});

// ---------------------------------------------------------------------------
// Invalid / rejected inputs
// ---------------------------------------------------------------------------

test("normalizeRelayUrl_empty_returns_null", () => {
  assert.equal(normalizeRelayUrl(""), null);
  assert.equal(normalizeRelayUrl("   "), null);
});

test("normalizeRelayUrl_bare_hostname_returns_null", () => {
  assert.equal(normalizeRelayUrl("relay.example.com"), null);
});

test("normalizeRelayUrl_ftp_scheme_returns_null", () => {
  assert.equal(normalizeRelayUrl("ftp://relay.example.com"), null);
});

test("normalizeRelayUrl_garbage_returns_null", () => {
  assert.equal(normalizeRelayUrl("not a url at all"), null);
});
