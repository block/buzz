import assert from "node:assert/strict";
import test from "node:test";

import {
  inviteErrorMessage,
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
