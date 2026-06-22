import assert from "node:assert/strict";
import test from "node:test";

import { isRelayUnreachableError } from "./relayError.ts";

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

test("isRelayUnreachableError: malformed-response message returns false", () => {
  // The backend relabels a reached-but-malformed 2xx body to this exact string
  // so it drops out of the unreachable bucket. Pin that the classifier agrees —
  // if the backend re-prefixes it, this catches the misroute.
  assert.equal(
    isRelayUnreachableError(
      "relay returned malformed response: not valid JSON",
    ),
    false,
  );
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
