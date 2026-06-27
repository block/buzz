import assert from "node:assert/strict";
import test from "node:test";

import {
  applyRemoteContextTimestamp,
  resolveEffectiveTimestamp,
  trimContextsToBudget,
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

// ── trimContextsToBudget ──────────────────────────────────────────────────────

const CLIENT_ID = "test-client-id";
const MSG_ID = "a".repeat(64);
const THREAD_ID = "b".repeat(64);

test("trimContextsToBudget_underBudget_returnsZeroAndLeavesContextsUnchanged", () => {
  const contexts = { [`msg:${MSG_ID}`]: 100 };
  // A very large budget — nothing should be evicted.
  const { evicted, fitsAfterTrim } = trimContextsToBudget(
    contexts,
    CLIENT_ID,
    1_000_000,
  );
  assert.equal(evicted, 0);
  assert.equal(fitsAfterTrim, true);
  assert.ok(`msg:${MSG_ID}` in contexts);
});

test("trimContextsToBudget_overBudget_evictsMsgEntriesOldestFirst", () => {
  // Build a contexts map that exceeds a tiny budget.
  // Three msg entries with timestamps 1 (oldest), 2, 3 (newest).
  const contexts = {
    [`msg:${MSG_ID}`]: 1,
    [`msg:${"c".repeat(64)}`]: 3,
    [`msg:${"d".repeat(64)}`]: 2,
  };
  const encoder = new TextEncoder();
  // Budget that requires evicting at least one entry.
  const budget =
    encoder.encode(JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts }))
      .length - 10;

  const { evicted, fitsAfterTrim } = trimContextsToBudget(
    contexts,
    CLIENT_ID,
    budget,
  );
  assert.ok(evicted >= 1, `expected at least 1 eviction, got ${evicted}`);
  assert.equal(fitsAfterTrim, true);
  // The oldest entry (ts=1) must be gone.
  assert.ok(
    !(`msg:${MSG_ID}` in contexts),
    "oldest msg entry should be evicted",
  );
  // Result must fit within budget.
  const resultSize = encoder.encode(
    JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts }),
  ).length;
  assert.ok(
    resultSize <= budget,
    `result ${resultSize} exceeds budget ${budget}`,
  );
});

test("trimContextsToBudget_channelKeysNeverEvicted", () => {
  // Fill with msg entries plus one channel key; budget forces eviction.
  const contexts = {};
  for (let i = 0; i < 50; i++) {
    contexts[`msg:${i.toString().padStart(64, "0")}`] = i;
  }
  contexts["channel:some-channel-id"] = 999;

  const encoder = new TextEncoder();
  const fullSize = encoder.encode(
    JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts }),
  ).length;
  const budget = Math.floor(fullSize / 2);

  const { fitsAfterTrim } = trimContextsToBudget(contexts, CLIENT_ID, budget);

  // Channel key must survive regardless of how many msg entries were evicted.
  assert.ok(
    "channel:some-channel-id" in contexts,
    "channel key must not be evicted",
  );
  assert.equal(fitsAfterTrim, true);
  const resultSize = encoder.encode(
    JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts }),
  ).length;
  assert.ok(
    resultSize <= budget,
    `result ${resultSize} exceeds budget ${budget}`,
  );
});

test("trimContextsToBudget_msgEvictedBeforeThread", () => {
  // One msg entry (older) and one thread entry (newer).
  // Budget forces exactly one eviction; msg must go first.
  const contexts = {
    [`msg:${MSG_ID}`]: 1,
    [`thread:${THREAD_ID}`]: 2,
  };
  const encoder = new TextEncoder();
  // Tight budget: remove exactly one entry.
  const oneEntrySize = encoder.encode(
    JSON.stringify({
      v: 1,
      client_id: CLIENT_ID,
      contexts: { [`thread:${THREAD_ID}`]: 2 },
    }),
  ).length;
  const budget = oneEntrySize + 5; // fits one entry, not two

  const { evicted, fitsAfterTrim } = trimContextsToBudget(
    contexts,
    CLIENT_ID,
    budget,
  );
  assert.equal(evicted, 1);
  assert.equal(fitsAfterTrim, true);
  assert.ok(
    !(`msg:${MSG_ID}` in contexts),
    "msg entry should be evicted before thread",
  );
  assert.ok(`thread:${THREAD_ID}` in contexts, "thread entry should survive");
});

test("trimContextsToBudget_emptyContexts_returnsZeroAndFits", () => {
  // Empty contexts: blob is just the skeleton — fits any reasonable budget.
  const contexts = {};
  const { evicted, fitsAfterTrim } = trimContextsToBudget(
    contexts,
    CLIENT_ID,
    1_000_000,
  );
  assert.equal(evicted, 0);
  assert.equal(fitsAfterTrim, true);
});

test("trimContextsToBudget_channelOnlyBlobExceedsBudget_fitsAfterTrimFalse", () => {
  // Channel keys cannot be evicted. If the channel-only skeleton exceeds the
  // budget, fitsAfterTrim must be false so the caller can suppress the publish.
  const contexts = {
    "channel:some-channel-id": 100,
  };
  const encoder = new TextEncoder();
  const skeletonSize = encoder.encode(
    JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts }),
  ).length;
  // Budget smaller than the channel-only skeleton — cannot be satisfied.
  const budget = skeletonSize - 1;

  const { evicted, fitsAfterTrim } = trimContextsToBudget(
    contexts,
    CLIENT_ID,
    budget,
  );
  assert.equal(evicted, 0, "no evictable entries exist");
  assert.equal(fitsAfterTrim, false, "channel-only blob still exceeds budget");
  // Channel key must still be present.
  assert.ok("channel:some-channel-id" in contexts);
});
