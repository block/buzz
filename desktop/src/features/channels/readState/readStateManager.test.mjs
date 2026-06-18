import assert from "node:assert/strict";
import test from "node:test";

import {
  applyRemoteContextTimestamp,
  resolveEffectiveTimestamp,
} from "./readStateManager.ts";

const threadKey = `thread:${"a".repeat(64)}`;
const channelKey = "channel-1";
const channelResolver = (ctx) =>
  ctx.startsWith("thread:") ? channelKey : null;

test("resolveEffectiveTimestamp returns own value when context has no parent", () => {
  const effectiveState = new Map([[channelKey, 200]]);
  const result = resolveEffectiveTimestamp({
    effectiveState,
    contextId: channelKey,
    parentResolver: channelResolver,
  });
  assert.equal(result, 200);
});

test("resolveEffectiveTimestamp inherits the channel frontier when it is newer than the thread", () => {
  // Channel-read clears its threads: marking the channel read at 300 must
  // dominate a thread last read at 100.
  const effectiveState = new Map([
    [threadKey, 100],
    [channelKey, 300],
  ]);
  const result = resolveEffectiveTimestamp({
    effectiveState,
    contextId: threadKey,
    parentResolver: channelResolver,
  });
  assert.equal(result, 300);
});

test("resolveEffectiveTimestamp keeps the thread frontier when it is newer than the channel", () => {
  const effectiveState = new Map([
    [threadKey, 400],
    [channelKey, 300],
  ]);
  const result = resolveEffectiveTimestamp({
    effectiveState,
    contextId: threadKey,
    parentResolver: channelResolver,
  });
  assert.equal(result, 400);
});

test("resolveEffectiveTimestamp returns the channel frontier when the thread was never read", () => {
  const effectiveState = new Map([[channelKey, 300]]);
  const result = resolveEffectiveTimestamp({
    effectiveState,
    contextId: threadKey,
    parentResolver: channelResolver,
  });
  assert.equal(result, 300);
});

test("resolveEffectiveTimestamp degrades to the thread's own value when the root is unresolvable", () => {
  // Resolver returns null (root not in the event graph) → own term only.
  const effectiveState = new Map([
    [threadKey, 100],
    [channelKey, 300],
  ]);
  const result = resolveEffectiveTimestamp({
    effectiveState,
    contextId: threadKey,
    parentResolver: () => null,
  });
  assert.equal(result, 100);
});

test("resolveEffectiveTimestamp degrades to own value when no resolver is set", () => {
  const effectiveState = new Map([
    [threadKey, 100],
    [channelKey, 300],
  ]);
  const result = resolveEffectiveTimestamp({
    effectiveState,
    contextId: threadKey,
    parentResolver: null,
  });
  assert.equal(result, 100);
});

test("resolveEffectiveTimestamp returns null when neither context nor parent has a value", () => {
  const result = resolveEffectiveTimestamp({
    effectiveState: new Map(),
    contextId: threadKey,
    parentResolver: channelResolver,
  });
  assert.equal(result, null);
});

test("applyRemoteContextTimestamp ignores older remote read markers from newer sync events", () => {
  const effectiveState = new Map([["channel-1", 200]]);
  const contextSourceCreatedAt = new Map([["channel-1", 10]]);

  const result = applyRemoteContextTimestamp({
    effectiveState,
    contextSourceCreatedAt,
    contextId: "channel-1",
    timestamp: 100,
    eventCreatedAt: 11,
  });

  assert.equal(result, "unchanged");
  assert.equal(effectiveState.get("channel-1"), 200);
  assert.equal(contextSourceCreatedAt.get("channel-1"), 11);
});

test("applyRemoteContextTimestamp advances to newer remote read markers", () => {
  const effectiveState = new Map([["channel-1", 100]]);
  const contextSourceCreatedAt = new Map([["channel-1", 10]]);

  const result = applyRemoteContextTimestamp({
    effectiveState,
    contextSourceCreatedAt,
    contextId: "channel-1",
    timestamp: 200,
    eventCreatedAt: 11,
  });

  assert.equal(result, "advanced");
  assert.equal(effectiveState.get("channel-1"), 200);
  assert.equal(contextSourceCreatedAt.get("channel-1"), 11);
});

test("applyRemoteContextTimestamp keeps read markers monotonic even if sync events arrive out of order", () => {
  const effectiveState = new Map([["channel-1", 100]]);
  const contextSourceCreatedAt = new Map([["channel-1", 11]]);

  const result = applyRemoteContextTimestamp({
    effectiveState,
    contextSourceCreatedAt,
    contextId: "channel-1",
    timestamp: 200,
    eventCreatedAt: 10,
  });

  assert.equal(result, "advanced");
  assert.equal(effectiveState.get("channel-1"), 200);
  assert.equal(contextSourceCreatedAt.get("channel-1"), 11);
});
