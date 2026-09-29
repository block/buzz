import assert from "node:assert/strict";
import test from "node:test";

import {
  getTextPayload,
  isRelayConnectionDegraded,
  sortEvents,
  toRelayFrames,
} from "./relayClientShared.ts";

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

test("toRelayFrames — a batched delivery yields each frame, in order", () => {
  const frames = [
    { type: "Text", data: '["EVENT","sub",{"id":"a"}]' },
    { type: "Text", data: '["EVENT","sub",{"id":"b"}]' },
  ];

  assert.deepEqual(toRelayFrames(frames), frames);
});

test("toRelayFrames — a single frame still delivers", () => {
  // The native plugin batches, but the unwrap must not depend on that: an
  // un-batched delivery is still a valid shape.
  const frame = { type: "Text", data: '["EOSE","sub"]' };

  assert.deepEqual(toRelayFrames(frame), [frame]);
});

test("toRelayFrames — close/error deliveries survive the unwrap", () => {
  const close = { type: "Close", data: { code: 1012, reason: "restart" } };

  assert.deepEqual(toRelayFrames([close]), [close]);
  assert.deepEqual(toRelayFrames(close), [close]);
});

test("getTextPayload rejects a batch — every client must unwrap first", () => {
  // The regression this pair guards: a client that forwards the raw delivery
  // to getTextPayload gets null and drops the batch silently, with no error.
  // Unwrapping first is what makes the same frames readable.
  const batch = [{ type: "Text", data: '["EOSE","sub"]' }];

  assert.equal(getTextPayload(batch), null);
  assert.equal(getTextPayload(toRelayFrames(batch)[0]), '["EOSE","sub"]');
});
