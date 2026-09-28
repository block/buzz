import assert from "node:assert/strict";
import test from "node:test";

import {
  isRelayMembershipDeniedError,
  isRelayUnreachableError,
} from "./relayError.ts";

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

test("isRelayMembershipDeniedError: bodyless 404 returns true", () => {
  // The exact string the membership gate produces. This is the case that
  // stranded invitees at the profile step (block/buzz#3544).
  assert.equal(
    isRelayMembershipDeniedError(new Error("relay returned 404 Not Found")),
    true,
  );
});

test("isRelayMembershipDeniedError: bodyless 404 as a string returns true", () => {
  assert.equal(
    isRelayMembershipDeniedError("relay returned 404 Not Found"),
    true,
  );
});

test("isRelayMembershipDeniedError: surrounding whitespace still matches", () => {
  assert.equal(
    isRelayMembershipDeniedError("  relay returned 404 Not Found\n"),
    true,
  );
});

test("isRelayMembershipDeniedError: suffixed 404 returns false", () => {
  // Every relay `api_error` carries {"error": "<msg>"}, which renders with a
  // `: <detail>` suffix. Those are different failures — a host with no
  // community bound is not a membership problem — and must not be
  // misreported as one.
  assert.equal(
    isRelayMembershipDeniedError(
      new Error(
        "relay returned 404 Not Found: relay: no community is configured for this host",
      ),
    ),
    false,
  );
});

test("isRelayMembershipDeniedError: other statuses return false", () => {
  assert.equal(
    isRelayMembershipDeniedError(new Error("relay returned 500")),
    false,
  );
  assert.equal(
    isRelayMembershipDeniedError(new Error("relay returned 403 Forbidden")),
    false,
  );
});

test("isRelayMembershipDeniedError: message-bearing denials return true", () => {
  for (const message of [
    "publish failed: You must be a relay member",
    "relay_membership_required",
    "restricted: not a relay member",
    "invalid: you are not a relay member",
  ]) {
    assert.equal(isRelayMembershipDeniedError(new Error(message)), true);
  }
});

test("isRelayMembershipDeniedError: unreachable relay returns false", () => {
  // A connectivity failure must route to the retry path, not to
  // MembershipDenied — the two are handled by different screens.
  assert.equal(
    isRelayMembershipDeniedError("relay unreachable: connection refused"),
    false,
  );
});

test("isRelayMembershipDeniedError: non-error inputs return false", () => {
  assert.equal(isRelayMembershipDeniedError(null), false);
  assert.equal(isRelayMembershipDeniedError(undefined), false);
  assert.equal(isRelayMembershipDeniedError(404), false);
  assert.equal(
    isRelayMembershipDeniedError({ message: "relay returned 404 Not Found" }),
    false,
  );
});
