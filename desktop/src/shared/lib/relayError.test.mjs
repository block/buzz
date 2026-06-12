import assert from "node:assert/strict";
import test from "node:test";

import {
  isRelayUnreachableError,
  relayErrorDetail,
  RELAY_UNREACHABLE_MESSAGE,
} from "./relayError.ts";

// ── isRelayUnreachableError ───────────────────────────────────────────────────

test("isRelayUnreachableError: Error with prefix returns true", () => {
  assert.equal(
    isRelayUnreachableError(new Error("relay unreachable: connection refused")),
    true,
  );
});

test("isRelayUnreachableError: string with prefix returns true", () => {
  assert.equal(
    isRelayUnreachableError("relay unreachable: 403 Forbidden"),
    true,
  );
});

test("isRelayUnreachableError: prefix alone (no detail) returns true", () => {
  assert.equal(isRelayUnreachableError("relay unreachable:"), true);
});

test("isRelayUnreachableError: unrelated Error returns false", () => {
  assert.equal(isRelayUnreachableError(new Error("network timeout")), false);
});

test("isRelayUnreachableError: unrelated string returns false", () => {
  assert.equal(isRelayUnreachableError("something went wrong"), false);
});

test("isRelayUnreachableError: null returns false", () => {
  assert.equal(isRelayUnreachableError(null), false);
});

test("isRelayUnreachableError: number returns false", () => {
  assert.equal(isRelayUnreachableError(42), false);
});

test("isRelayUnreachableError: plain object returns false", () => {
  assert.equal(
    isRelayUnreachableError({ message: "relay unreachable: oops" }),
    false,
  );
});

// ── relayErrorDetail ──────────────────────────────────────────────────────────

test("relayErrorDetail: strips prefix and trims for Error", () => {
  const err = new Error("relay unreachable:   connection refused  ");
  assert.equal(relayErrorDetail(err), "connection refused");
});

test("relayErrorDetail: strips prefix and trims for string", () => {
  assert.equal(
    relayErrorDetail("relay unreachable: 403 Forbidden from Cloudflare Access"),
    "403 Forbidden from Cloudflare Access",
  );
});

test("relayErrorDetail: prefix with no detail returns RELAY_UNREACHABLE_MESSAGE", () => {
  assert.equal(
    relayErrorDetail("relay unreachable:"),
    RELAY_UNREACHABLE_MESSAGE,
  );
});

test("relayErrorDetail: unrelated Error returns generic message", () => {
  const detail = relayErrorDetail(new Error("something else"));
  assert.equal(detail, RELAY_UNREACHABLE_MESSAGE);
});

test("relayErrorDetail: null returns generic message", () => {
  const detail = relayErrorDetail(null);
  assert.equal(detail, RELAY_UNREACHABLE_MESSAGE);
});
