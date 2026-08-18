import assert from "node:assert/strict";
import test from "node:test";

import { isRelayConnectionDegraded, sortEvents } from "./relayClientShared.ts";

function event(id, createdAt) {
  return {
    id,
    pubkey: "pubkey",
    created_at: createdAt,
    kind: 9,
    tags: [],
    content: "",
    sig: "sig",
  };
}

test("sortEvents — same-second events sort by id, order-independent", () => {
  const a = event("aaa", 100);
  const b = event("bbb", 100);
  const c = event("ccc", 101);

  const forward = sortEvents([a, b, c]).map((e) => e.id);
  const shuffled = sortEvents([c, b, a]).map((e) => e.id);

  // Stable (created_at, id) order regardless of input order, matching the
  // cache sort (sortMessages) and the relay's id-ASC same-second tiebreak.
  assert.deepEqual(forward, ["aaa", "bbb", "ccc"]);
  assert.deepEqual(shuffled, ["aaa", "bbb", "ccc"]);
});

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
