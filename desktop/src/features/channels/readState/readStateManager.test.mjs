import assert from "node:assert/strict";
import test from "node:test";

import {
  ReadStateManager,
  applyRemoteContextTimestamp,
  resolveEffectiveTimestamp,
  splitContextsIntoBudgetedSlots,
  trimContextsToBudget,
} from "./readStateManager.ts";

// ── ReadStateManager integration helpers ─────────────────────────────────────
// Provide browser globals required by ReadStateManager (localStorage,
// window.setTimeout/clearTimeout). Each test that uses ReadStateManager
// constructs a fresh in-memory store so tests are isolated.

function makeLocalStorage() {
  const store = new Map();
  const writes = [];
  return {
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => {
      writes.push([key, value]);
      store.set(key, value);
    },
    removeItem: (key) => store.delete(key),
    writes,
  };
}

function makeFakeTimers() {
  let nextId = 1;
  let now = 0;
  const timers = new Map();

  function runThrough(targetTime) {
    while (true) {
      const next = [...timers.entries()]
        .filter(([, timer]) => timer.dueAt <= targetTime)
        .sort(
          ([firstId, first], [secondId, second]) =>
            first.dueAt - second.dueAt || firstId - secondId,
        )[0];
      if (!next) break;

      const [id, timer] = next;
      timers.delete(id);
      now = timer.dueAt;
      timer.fn();
    }
    now = targetTime;
  }

  return {
    setTimeout(fn, delay = 0) {
      const id = nextId++;
      timers.set(id, { fn, dueAt: now + delay });
      return id;
    },
    clearTimeout(id) {
      timers.delete(id);
    },
    advanceBy(ms) {
      runThrough(now + ms);
    },
    runAll() {
      while (timers.size > 0) {
        const nextDueAt = Math.min(
          ...[...timers.values()].map((timer) => timer.dueAt),
        );
        runThrough(nextDueAt);
      }
    },
    get size() {
      return timers.size;
    },
  };
}

function makeFakeRelay() {
  return {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeLive: () => () => {},
  };
}

// Install browser globals required by ReadStateManager. window.localStorage is
// replaced per-test for isolation; the bare `localStorage` global proxies to it.
{
  const ls = makeLocalStorage();
  const windowEvents = new EventTarget();
  const documentEvents = new EventTarget();
  globalThis.window = {
    localStorage: ls,
    clearTimeout: (id) => clearTimeout(id),
    setTimeout: (fn, ms) => setTimeout(fn, ms),
    addEventListener: (...args) => windowEvents.addEventListener(...args),
    removeEventListener: (...args) => windowEvents.removeEventListener(...args),
    dispatchEvent: (...args) => windowEvents.dispatchEvent(...args),
  };
  globalThis.document = {
    visibilityState: "visible",
    addEventListener: (...args) => documentEvents.addEventListener(...args),
    removeEventListener: (...args) =>
      documentEvents.removeEventListener(...args),
    dispatchEvent: (...args) => documentEvents.dispatchEvent(...args),
  };
  // Ensure bare `localStorage` always proxies to window.localStorage.
  Object.defineProperty(globalThis, "localStorage", {
    get: () => globalThis.window.localStorage,
    configurable: true,
  });
}

const threadKey = `thread:${"a".repeat(64)}`;
const channelKey = "channel-1";
const channelResolver = (ctx) =>
  ctx.startsWith("thread:") ? channelKey : null;

// ── ReadStateManager local persistence ────────────────────────────────────────

function withFakeTimers() {
  const timers = makeFakeTimers();
  const originalSetTimeout = globalThis.window.setTimeout;
  const originalClearTimeout = globalThis.window.clearTimeout;
  globalThis.window.setTimeout = (fn, ms) => timers.setTimeout(fn, ms);
  globalThis.window.clearTimeout = (id) => timers.clearTimeout(id);
  return {
    timers,
    restore() {
      globalThis.window.setTimeout = originalSetTimeout;
      globalThis.window.clearTimeout = originalClearTimeout;
    },
  };
}

test("advanceContext burst coalesces local persistence into one write", () => {
  const storage = makeLocalStorage();
  globalThis.window.localStorage = storage;
  const { timers, restore } = withFakeTimers();
  const manager = new ReadStateManager("1".repeat(64), makeFakeRelay());
  const baselineWrites = storage.writes.length;

  try {
    manager.seedContextRead("channel-1", 100);
    manager.seedContextRead("channel-2", 200);
    manager.seedContextRead("channel-3", 300);

    assert.equal(timers.size, 1, "burst should leave one trailing timer");
    assert.equal(storage.writes.length, baselineWrites);

    timers.runAll();
    assert.equal(
      storage.writes.length - baselineWrites,
      3,
      "one writeStoredReadState call writes its three blobs once",
    );
  } finally {
    manager.destroy();
    restore();
  }
});

test("sustained advances persist within each one-second window", () => {
  const storage = makeLocalStorage();
  globalThis.window.localStorage = storage;
  const { timers, restore } = withFakeTimers();
  const manager = new ReadStateManager("5".repeat(64), makeFakeRelay());
  const baselineWrites = storage.writes.length;

  try {
    manager.seedContextRead("channel-1", 100);

    for (let timestamp = 200; timestamp <= 3_200; timestamp += 200) {
      timers.advanceBy(200);
      manager.seedContextRead("channel-1", timestamp);

      if (timestamp % 1_000 === 0) {
        assert.equal(
          storage.writes.length - baselineWrites,
          (timestamp / 1_000) * 3,
          "latest state should persist once per one-second window",
        );
      }
    }

    const contexts = JSON.parse(
      storage.getItem(`buzz.channel-read-state.v2:${"5".repeat(64)}`),
    );
    assert.equal(contexts["channel-1"], new Date(2_800_000).toISOString());
    assert.equal(timers.size, 1, "latest advance should open the next window");
  } finally {
    manager.destroy();
    restore();
  }
});

test("visibility hidden flushes pending local state", () => {
  const storage = makeLocalStorage();
  globalThis.window.localStorage = storage;
  const { timers, restore } = withFakeTimers();
  const manager = new ReadStateManager("2".repeat(64), makeFakeRelay());
  const baselineWrites = storage.writes.length;

  try {
    manager.seedContextRead("channel-1", 100);
    assert.equal(storage.writes.length, baselineWrites);

    globalThis.document.visibilityState = "hidden";
    globalThis.document.dispatchEvent(new Event("visibilitychange"));

    assert.equal(timers.size, 0, "flush should cancel the trailing timer");
    assert.equal(storage.writes.length - baselineWrites, 3);
    const contexts = JSON.parse(
      storage.getItem(`buzz.channel-read-state.v2:${"2".repeat(64)}`),
    );
    assert.equal(contexts["channel-1"], new Date(100_000).toISOString());
  } finally {
    globalThis.document.visibilityState = "visible";
    manager.destroy();
    restore();
  }
});

test("hydrateFromLocalStorage persists immediately", () => {
  const storage = makeLocalStorage();
  const pubkey = "3".repeat(64);
  storage.setItem(
    `buzz.channel-read-state.v2:${pubkey}`,
    JSON.stringify({ "channel-1": new Date(100_000).toISOString() }),
  );
  globalThis.window.localStorage = storage;
  const { timers, restore } = withFakeTimers();
  const manager = new ReadStateManager(pubkey, makeFakeRelay());
  const baselineWrites = storage.writes.length;

  try {
    manager.hydrateFromLocalStorage();

    assert.equal(timers.size, 0);
    assert.equal(storage.writes.length - baselineWrites, 3);
    assert.equal(manager.getOwnTimestamp("channel-1"), 100);
  } finally {
    manager.destroy();
    restore();
  }
});

test("publish flushes pending local state first", async () => {
  const storage = makeLocalStorage();
  globalThis.window.localStorage = storage;
  const { timers, restore } = withFakeTimers();
  const manager = new ReadStateManager("4".repeat(64), makeFakeRelay());
  const baselineWrites = storage.writes.length;

  try {
    manager.seedContextRead("channel-1", 100);
    manager.fetchOwnBlobBeforePublish = async () => {};

    await manager.publish();

    assert.equal(timers.size, 0);
    assert.equal(storage.writes.length - baselineWrites, 3);
  } finally {
    manager.destroy();
    restore();
  }
});

test("destroyed manager cannot persist after an in-flight fetch resolves", async () => {
  const storage = makeLocalStorage();
  globalThis.window.localStorage = storage;
  const { timers, restore } = withFakeTimers();
  const pubkey = "6".repeat(64);
  let resolveFetch;
  const fetchPending = new Promise((resolve) => {
    resolveFetch = resolve;
  });
  const staleManager = new ReadStateManager(pubkey, {
    ...makeFakeRelay(),
    fetchEvents: () => fetchPending,
  });

  try {
    const initialization = staleManager.initialize();
    staleManager.seedContextRead("channel-1", 100);
    staleManager.destroy();

    const replacementManager = new ReadStateManager(pubkey, makeFakeRelay());
    replacementManager.seedContextRead("channel-1", 200);
    timers.advanceBy(1_000);
    const writesAfterReplacement = storage.writes.length;

    resolveFetch([]);
    await initialization;
    timers.runAll();

    assert.equal(
      storage.writes.length,
      writesAfterReplacement,
      "the stale manager must not schedule persistence after destruction",
    );
    const contexts = JSON.parse(
      storage.getItem(`buzz.channel-read-state.v2:${pubkey}`),
    );
    assert.equal(
      contexts["channel-1"],
      new Date(200_000).toISOString(),
      "the stale manager must not overwrite the replacement manager",
    );
    replacementManager.destroy();
  } finally {
    staleManager.destroy();
    restore();
  }
});

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

// ── splitContextsIntoBudgetedSlots ────────────────────────────────────────────

// Build a channel key that is ~70 bytes in the JSON blob:
// `"channel-<64-hex>":1` ≈ 70 bytes including quotes, colon, comma.
const makeChannelKey = (n) => `channel-${n.toString().padStart(64, "0")}`;
const makeThreadKey = (n) => `thread:${n.toString().padStart(64, "0")}`;
const makeMsgKey = (n) => `msg:${n.toString().padStart(64, "0")}`;

// Compute the byte size of a single-slot blob with the given contexts.
const blobSize = (clientId, contexts) => {
  const encoder = new TextEncoder();
  return encoder.encode(JSON.stringify({ v: 1, client_id: clientId, contexts }))
    .length;
};

let slotCounter = 0;
const deterministicSlotId = () =>
  `slot-${(++slotCounter).toString().padStart(4, "0")}`;

test("splitContextsIntoBudgetedSlots_fitsInOneSlot_returnsSingleSlot", () => {
  // 3 channel keys — easily fits in one slot with a generous budget.
  const channelEntries = [
    [makeChannelKey(1), 100],
    [makeChannelKey(2), 200],
    [makeChannelKey(3), 300],
  ];
  const result = splitContextsIntoBudgetedSlots({
    channelEntries,
    threadMsgEntries: [],
    clientId: CLIENT_ID,
    initialSlotCount: 1,
    maxSlots: 8,
    maxBytes: 1_000_000,
    slotIdGenerator: deterministicSlotId,
  });

  assert.ok(result !== null, "should succeed");
  assert.equal(result.slots.length, 1, "single slot");
  assert.equal(result.extraSlotIds.length, 0, "no extra slots allocated");
  // All channel keys present in slot 0.
  for (const [key] of channelEntries) {
    assert.ok(key in result.slots[0], `${key} should be in slot 0`);
  }
});

test("splitContextsIntoBudgetedSlots_requiresGrowth_allocatesExtraSlot", () => {
  // Build enough channel keys that a single slot overflows a tight budget
  // but two slots fit.
  const channelEntries = [];
  for (let i = 0; i < 20; i++) {
    channelEntries.push([makeChannelKey(i), i + 1]);
  }
  const encoder = new TextEncoder();
  // Budget that fits ~10 channel keys but not 20.
  const tenKeyContexts = Object.fromEntries(channelEntries.slice(0, 10));
  const tenKeySize = encoder.encode(
    JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts: tenKeyContexts }),
  ).length;
  const budget = tenKeySize + 50; // fits 10 but not 20

  const result = splitContextsIntoBudgetedSlots({
    channelEntries,
    threadMsgEntries: [],
    clientId: CLIENT_ID,
    initialSlotCount: 1,
    maxSlots: 8,
    maxBytes: budget,
    slotIdGenerator: deterministicSlotId,
  });

  assert.ok(result !== null, "should succeed with 2 slots");
  assert.equal(result.slots.length, 2, "two slots");
  assert.equal(result.extraSlotIds.length, 1, "one extra slot allocated");
  // All 20 keys present across both slots.
  const allKeys = new Set([
    ...Object.keys(result.slots[0]),
    ...Object.keys(result.slots[1]),
  ]);
  for (const [key] of channelEntries) {
    assert.ok(allKeys.has(key), `${key} should appear in some slot`);
  }
  // Each slot fits within budget.
  for (const slotContexts of result.slots) {
    const size = encoder.encode(
      JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts: slotContexts }),
    ).length;
    assert.ok(size <= budget, `slot size ${size} exceeds budget ${budget}`);
  }
});

test("splitContextsIntoBudgetedSlots_exceedsMaxSlots_returnsNull", () => {
  // Build enough channel keys that even maxSlots=2 can't fit them with a
  // very tight budget (1 byte — nothing can fit).
  const channelEntries = [[makeChannelKey(1), 1]];
  const result = splitContextsIntoBudgetedSlots({
    channelEntries,
    threadMsgEntries: [],
    clientId: CLIENT_ID,
    initialSlotCount: 1,
    maxSlots: 2,
    maxBytes: 1, // impossibly small
    slotIdGenerator: deterministicSlotId,
  });

  assert.equal(result, null, "should return null when max slots exceeded");
});

test("splitContextsIntoBudgetedSlots_includesThreadMsgInPrimarySlot", () => {
  // Channel key in slot 0; thread and msg entries should also land in slot 0.
  const channelEntries = [[makeChannelKey(1), 100]];
  const threadMsgEntries = [
    [makeThreadKey(1), 200],
    [makeMsgKey(1), 300],
  ];

  const result = splitContextsIntoBudgetedSlots({
    channelEntries,
    threadMsgEntries,
    clientId: CLIENT_ID,
    initialSlotCount: 1,
    maxSlots: 8,
    maxBytes: 1_000_000,
    slotIdGenerator: deterministicSlotId,
  });

  assert.ok(result !== null, "should succeed");
  assert.equal(result.slots.length, 1);
  // Channel key in slot 0.
  assert.ok(makeChannelKey(1) in result.slots[0], "channel key in slot 0");
  // Thread and msg entries in slot 0.
  assert.ok(makeThreadKey(1) in result.slots[0], "thread key in slot 0");
  assert.ok(makeMsgKey(1) in result.slots[0], "msg key in slot 0");
});

test("splitContextsIntoBudgetedSlots_threadMsgTrimmedWhenPrimarySlotOverBudget", () => {
  // Channel key fills the primary slot to near-budget. Thread/msg entries
  // added to slot 0 would overflow — trimContextsToBudget must evict them.
  const channelEntries = [[makeChannelKey(1), 100]];
  // Compute the size of a blob with just the channel key.
  const channelOnlyContexts = { [makeChannelKey(1)]: 100 };
  const channelOnlySize = blobSize(CLIENT_ID, channelOnlyContexts);
  // Budget = channel-only size + 5 bytes: fits the channel key but not
  // an additional thread/msg entry (~70+ bytes each).
  const budget = channelOnlySize + 5;

  const threadMsgEntries = [[makeThreadKey(1), 200]];

  const result = splitContextsIntoBudgetedSlots({
    channelEntries,
    threadMsgEntries,
    clientId: CLIENT_ID,
    initialSlotCount: 1,
    maxSlots: 8,
    maxBytes: budget,
    slotIdGenerator: deterministicSlotId,
  });

  assert.ok(result !== null, "should succeed");
  // Channel key must survive (never evicted by trimContextsToBudget).
  assert.ok(makeChannelKey(1) in result.slots[0], "channel key survives");
  // Thread entry must be evicted (doesn't fit within budget).
  assert.ok(
    !(makeThreadKey(1) in result.slots[0]),
    "thread key evicted to fit budget",
  );
  // Slot 0 must fit within budget.
  const size = blobSize(CLIENT_ID, result.slots[0]);
  assert.ok(size <= budget, `slot 0 size ${size} exceeds budget ${budget}`);
});

// ── ReadStateManager.publish — no-op suppression in split mode ────────────────

// Verify that publishSplitSlots returns early (no relay writes) when the
// union of all slot contexts is identical to lastPublishedContexts.
//
// Strategy: construct a ReadStateManager with enough channel keys to force
// split mode, then mock publishOneSlot (private, accessed via bracket notation)
// to avoid tauri calls while still simulating its effect on lastPublishedContexts.
// Call publish() twice with the same effectiveState and assert that
// publishOneSlot is called only on the first publish (no-op on the second).
test("publishSplitSlots_noopSuppression_skipsWhenUnchanged", async () => {
  // Isolate localStorage so slot IDs don't leak between tests.
  globalThis.window.localStorage = makeLocalStorage();

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeLive: () => () => {},
  };

  const pubkey = "b".repeat(64);
  const mgr = new ReadStateManager(pubkey, fakeRelay);

  // Add enough channel keys to exceed the 32KB single-slot budget.
  // Each key is ~70 bytes in the blob; 700 keys ≈ 49KB > 32KB.
  const ts = 1_000_000;
  for (let i = 0; i < 700; i++) {
    const channelId = `channel-${i.toString().padStart(64, "0")}`;
    mgr.markContextRead(channelId, ts);
  }

  // Confirm split mode: currentContexts() must return null.
  assert.equal(
    mgr.currentContexts(),
    null,
    "precondition: 700 channel keys must exceed single-slot budget",
  );

  // Replace publishOneSlot with a stub that records calls and simulates the
  // lastPublishedContexts merge (the only side-effect the no-op check depends
  // on). This avoids tauri (nip44EncryptToSelf / signRelayEvent) while keeping
  // the suppression logic under test.
  let publishOneSlotCallCount = 0;
  mgr.publishOneSlot = async (_slotId, contexts) => {
    publishOneSlotCallCount++;
    for (const [key, tsVal] of Object.entries(contexts)) {
      mgr.lastPublishedContexts[key] = tsVal;
    }
  };

  // First publish: contexts differ from lastPublishedContexts ({}) → must publish.
  await mgr.publish();
  const callsAfterFirst = publishOneSlotCallCount;
  assert.ok(callsAfterFirst > 0, "first publish must call publishOneSlot");

  // Second publish with identical effectiveState: union equals lastPublishedContexts
  // → no-op suppression must fire → publishOneSlot must NOT be called again.
  await mgr.publish();
  assert.equal(
    publishOneSlotCallCount,
    callsAfterFirst,
    "second publish with unchanged state must not call publishOneSlot (no-op suppression)",
  );

  mgr.destroy();
});

// ── ReadStateManager — self-echo drop ─────────────────────────────────────────

// The live subscription (authors=[us]) echoes back every event we publish.
// handleIncomingEvent must drop those echoes by id BEFORE the decrypt/parse
// step: with hundreds of channels a blob is tens of KB, so decrypting our own
// echo on every read was a large recurring cost. Strategy mirrors the split-
// mode test above: stub the private parseEvent seam to count decrypt attempts.
test("handleIncomingEvent_dropsSelfEchoBeforeDecrypt", async () => {
  globalThis.window.localStorage = makeLocalStorage();

  const pubkey = "c".repeat(64);
  const mgr = new ReadStateManager(pubkey, makeFakeRelay());

  let parseCount = 0;
  mgr.parseEvent = async () => {
    parseCount++;
    return null;
  };

  const makeEvent = (id) => ({
    id,
    pubkey,
    kind: 30078,
    created_at: 1_000,
    content: "ciphertext",
    tags: [
      ["d", "read-state:slot"],
      ["t", "read-state"],
    ],
  });

  // An event we published ourselves: echo must be dropped without a parse.
  const ownId = "e".repeat(64);
  mgr.rememberPublishedId(ownId);
  await mgr.handleIncomingEvent(makeEvent(ownId));
  assert.equal(parseCount, 0, "self-echo must not reach decrypt/parse");

  // The drop consumes the remembered id — a replayed duplicate (e.g. from a
  // reconnect catch-up) goes through the normal parse path.
  await mgr.handleIncomingEvent(makeEvent(ownId));
  assert.equal(parseCount, 1, "second delivery of same id must parse");

  // An event from another client of the same pubkey must always parse.
  await mgr.handleIncomingEvent(makeEvent("f".repeat(64)));
  assert.equal(parseCount, 2, "foreign-client event must parse");

  mgr.destroy();
});

// The remembered-id set must stay bounded even if publishes fail (a failed
// publish leaves an id that is never echoed back, so nothing deletes it).
test("rememberPublishedId_evictsOldestBeyondCap", () => {
  globalThis.window.localStorage = makeLocalStorage();

  const mgr = new ReadStateManager("d".repeat(64), makeFakeRelay());

  const total = 100; // beyond the 64-id cap
  for (let i = 0; i < total; i++) {
    mgr.rememberPublishedId(`id-${i}`);
  }
  const ids = mgr.recentlyPublishedIds;
  assert.equal(ids.size, 64, "set must be capped");
  assert.ok(!ids.has("id-0"), "oldest id must be evicted");
  assert.ok(ids.has(`id-${total - 1}`), "newest id must be retained");

  mgr.destroy();
});
