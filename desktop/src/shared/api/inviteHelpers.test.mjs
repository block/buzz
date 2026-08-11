import assert from "node:assert/strict";
import test from "node:test";

import {
  identityHandoffTerminalReason,
  inviteErrorMessage,
  isIdentityHandoffCode,
  isIdentityHandoffMismatchError,
  isInviteExhaustedError,
  isInviteExpiredError,
  relayHttpFromWs,
} from "./inviteHelpers.ts";

test("relayHttpFromWs maps secure and local relay schemes", () => {
  assert.equal(
    relayHttpFromWs("wss://relay.example/path"),
    "https://relay.example/path",
  );
  assert.equal(relayHttpFromWs("ws://localhost:7000"), "http://localhost:7000");
});

test("relayHttpFromWs rejects unexpected schemes", () => {
  assert.throws(
    () => relayHttpFromWs("https://relay.example"),
    /Expected ws:\/\/ or wss:\/\//,
  );
  assert.throws(
    () => relayHttpFromWs("relay.example"),
    /Expected ws:\/\/ or wss:\/\//,
  );
});

test("invite expiry sentinel is recognized without hiding other errors", () => {
  assert.equal(isInviteExpiredError(new Error("invite_expired")), true);
  assert.equal(isInviteExpiredError(new Error("invite_invalid")), false);
  assert.equal(
    inviteErrorMessage("network unavailable"),
    "network unavailable",
  );
});

test("invite exhaustion sentinel is recognized distinctly from expiry", () => {
  assert.equal(isInviteExhaustedError(new Error("invite_exhausted")), true);
  assert.equal(isInviteExhaustedError(new Error("invite_expired")), false);
  assert.equal(isInviteExhaustedError(new Error("invite_invalid")), false);
});

test("identity handoff recognition accepts only the canonical lowercase v3 shape", () => {
  assert.equal(isIdentityHandoffCode(`v3.${"a".repeat(64)}`), true);
  for (const code of [
    `v3.${"A".repeat(64)}`,
    `v3.${"a".repeat(63)}`,
    `v3.${"a".repeat(65)}`,
    `v3.${"g".repeat(64)}`,
    `prefix-v3.${"a".repeat(64)}`,
    `v2.${"a".repeat(64)}`,
  ]) {
    assert.equal(isIdentityHandoffCode(code), false, code);
  }
});

test("identity handoff mismatch and terminal outcomes remain typed", () => {
  assert.equal(
    isIdentityHandoffMismatchError(new Error("invite_identity_mismatch")),
    true,
  );
  assert.equal(
    identityHandoffTerminalReason(new Error("invite_expired")),
    "expired",
  );
  assert.equal(
    identityHandoffTerminalReason(new Error("invite_superseded")),
    "superseded",
  );
  assert.equal(
    identityHandoffTerminalReason(new Error("invite_invalidated")),
    "invalidated",
  );
  assert.equal(
    identityHandoffTerminalReason(new Error("invite_client_upgrade_required")),
    "upgrade-required",
  );
  assert.equal(
    identityHandoffTerminalReason(new Error("invite_invalid")),
    "invalid",
  );
  assert.equal(
    identityHandoffTerminalReason(new Error("invite_already_claimed")),
    "already-claimed",
  );
  assert.equal(identityHandoffTerminalReason(new Error("network down")), null);
});
