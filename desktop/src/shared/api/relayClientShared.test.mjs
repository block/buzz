import assert from "node:assert/strict";
import test from "node:test";

import { isRelayConnectionDegraded } from "./relayClientShared.ts";

test("isRelayConnectionDegraded — healthy states are not degraded", () => {
  assert.equal(isRelayConnectionDegraded("idle"), false);
  assert.equal(isRelayConnectionDegraded("connecting"), false);
  assert.equal(isRelayConnectionDegraded("connected"), false);
});

test("isRelayConnectionDegraded — non-healthy states are degraded", () => {
  assert.equal(isRelayConnectionDegraded("reconnecting"), true);
  assert.equal(isRelayConnectionDegraded("stalled"), true);
  assert.equal(isRelayConnectionDegraded("disconnected"), true);
});
