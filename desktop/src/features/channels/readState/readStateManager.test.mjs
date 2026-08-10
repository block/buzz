import assert from "node:assert/strict";
import test from "node:test";

import {
  ReadStateManager,
  applyRemoteContextTimestamp,
  resolveEffectiveTimestamp,
  splitContextsIntoBudgetedSlots,
  trimContextsToBudget,
} from "./readStateManager.ts";

import { createFencedSubscription } from "../../../shared/api/relayClientShared.ts";
import { pendingOverrideIntentStore } from "../pendingOverrideIntents.ts";
import { forcedUnreadStore } from "../forcedUnreadStore.ts";
import { createDrainOutcomeHandler } from "../useUnreadChannelsHelpers.ts";

// ── Shared test helpers ───────────────────────────────────────────────────────

function makeLocalStorage() {
  const store = new Map();
  return {
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, value),
    removeItem: (key) => store.delete(key),
  };
}

function makeFenceHandle({
  eose = true,
  lapsesAfterEose = false,
  lapseBeforeEose = false,
} = {}) {
  let lapsed = lapseBeforeEose;
  let resolveEstablished;
  const established = new Promise((r) => {
    resolveEstablished = r;
  });
  if (lapseBeforeEose) {
    resolveEstablished();
  } else if (eose) {
    resolveEstablished();
    if (lapsesAfterEose) lapsed = true;
  }
  return {
    established,
    get lapsed() {
      return lapsed;
    },
    unsubscribe: async () => {},
    _lapse() {
      lapsed = true;
    },
  };
}

function makeFakeRelay(overrides = {}) {
  return {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    ...overrides,
  };
}

function makeFakeTimers() {
  const scheduled = new Map();
  let nextSyntheticId = 1_000_000;
  const origSetTimeout = globalThis.window.setTimeout;
  const origClearTimeout = globalThis.window.clearTimeout;
  globalThis.window.setTimeout = (fn, ms) => {
    if (ms > 0) {
      // Positive-delay timers are purely synthetic: store the callback in the
      // map and return a synthetic ID without touching the native scheduler.
      // Tests fire them explicitly via the returned fn; the process never lingers.
      const id = nextSyntheticId++;
      scheduled.set(id, { fn, ms });
      return id;
    }
    // Zero-delay work delegates to the native scheduler so async continuations
    // and micro-queues flush correctly.
    return origSetTimeout(fn, ms);
  };
  globalThis.window.clearTimeout = (id) => {
    if (scheduled.has(id)) {
      scheduled.delete(id);
    } else {
      origClearTimeout(id);
    }
  };
  return {
    scheduled,
    origSetTimeout,
    restore() {
      globalThis.window.setTimeout = origSetTimeout;
      globalThis.window.clearTimeout = origClearTimeout;
    },
  };
}

// Install browser globals required by ReadStateManager. window.localStorage is
// replaced per-test for isolation; the bare `localStorage` global proxies to it.
{
  const ls = makeLocalStorage();
  if (typeof globalThis.window === "undefined") {
    globalThis.window = {
      localStorage: ls,
      clearTimeout: (id) => clearTimeout(id),
      setTimeout: (fn, ms) => setTimeout(fn, ms),
    };
  } else {
    globalThis.window.localStorage = ls;
    if (!globalThis.window.clearTimeout) {
      globalThis.window.clearTimeout = (id) => clearTimeout(id);
      globalThis.window.setTimeout = (fn, ms) => setTimeout(fn, ms);
    }
  }
  Object.defineProperty(globalThis, "localStorage", {
    get: () => globalThis.window.localStorage,
    configurable: true,
  });
}

/**
 * Wire the production `createDrainOutcomeHandler` onto `manager.onDrainOutcome`.
 * Returns { forcedUnreadMap, pendingSnapshots, versionBumps, teardown }.
 */
function wireDrainCallback(manager, pubkey) {
  const forcedUnreadMap = forcedUnreadStore.read(pubkey);
  const pendingSnapshots = new Map();
  let versionBumps = 0;
  manager.onDrainOutcome = createDrainOutcomeHandler(
    { current: forcedUnreadMap },
    pendingSnapshots,
    pubkey,
    () => {
      versionBumps++;
    },
  );
  return {
    forcedUnreadMap,
    pendingSnapshots,
    get versionBumps() {
      return versionBumps;
    },
    teardown() {
      manager.onDrainOutcome = null;
    },
  };
}

const threadKey = `thread:${"a".repeat(64)}`;
const channelKey = "channel-1";
const channelResolver = (ctx) =>
  ctx.startsWith("thread:") ? channelKey : null;

// biome-ignore format: compact table rows
const resolveEffectiveTimestampCases = [
  { name: "returns own value when context has no parent", effectiveState: new Map([[channelKey, 200]]), contextId: channelKey, resolver: channelResolver, expected: 200 },
  { name: "inherits the channel frontier when it is newer than the thread", effectiveState: new Map([[threadKey, 100], [channelKey, 300]]), contextId: threadKey, resolver: channelResolver, expected: 300 },
  { name: "keeps the thread frontier when it is newer than the channel", effectiveState: new Map([[threadKey, 400], [channelKey, 300]]), contextId: threadKey, resolver: channelResolver, expected: 400 },
  { name: "returns the channel frontier when the thread was never read", effectiveState: new Map([[channelKey, 300]]), contextId: threadKey, resolver: channelResolver, expected: 300 },
  { name: "degrades to the thread's own value when the root is unresolvable", effectiveState: new Map([[threadKey, 100], [channelKey, 300]]), contextId: threadKey, resolver: () => null, expected: 100 },
  { name: "degrades to own value when no resolver is set", effectiveState: new Map([[threadKey, 100], [channelKey, 300]]), contextId: threadKey, resolver: null, expected: 100 },
  { name: "returns null when neither context nor parent has a value", effectiveState: new Map(), contextId: threadKey, resolver: channelResolver, expected: null },
];
for (const row of resolveEffectiveTimestampCases) {
  test(`resolveEffectiveTimestamp ${row.name}`, () => {
    assert.equal(
      resolveEffectiveTimestamp({
        effectiveState: row.effectiveState,
        contextId: row.contextId,
        parentResolver: row.resolver,
      }),
      row.expected,
    );
  });
}

// biome-ignore format: compact table rows
const applyRemoteContextTimestampCases = [
  { name: "ignores older remote read markers from newer sync events", initEffective: 200, initSourceCreatedAt: 10, timestamp: 100, eventCreatedAt: 11, expectedResult: "unchanged", expectedEffective: 200, expectedSourceCreatedAt: 11 },
  { name: "advances to newer remote read markers", initEffective: 100, initSourceCreatedAt: 10, timestamp: 200, eventCreatedAt: 11, expectedResult: "advanced", expectedEffective: 200, expectedSourceCreatedAt: 11 },
  { name: "keeps read markers monotonic even if sync events arrive out of order", initEffective: 100, initSourceCreatedAt: 11, timestamp: 200, eventCreatedAt: 10, expectedResult: "advanced", expectedEffective: 200, expectedSourceCreatedAt: 11 },
];
for (const row of applyRemoteContextTimestampCases) {
  test(`applyRemoteContextTimestamp ${row.name}`, () => {
    const effectiveState = new Map([["channel-1", row.initEffective]]);
    const contextSourceCreatedAt = new Map([
      ["channel-1", row.initSourceCreatedAt],
    ]);
    const result = applyRemoteContextTimestamp({
      effectiveState,
      contextSourceCreatedAt,
      contextId: "channel-1",
      timestamp: row.timestamp,
      eventCreatedAt: row.eventCreatedAt,
    });
    assert.equal(result, row.expectedResult);
    assert.equal(effectiveState.get("channel-1"), row.expectedEffective);
    assert.equal(
      contextSourceCreatedAt.get("channel-1"),
      row.expectedSourceCreatedAt,
    );
  });
}

// ── trimContextsToBudget ──────────────────────────────────────────────────────

const CLIENT_ID = "test-client-id";
const MSG_ID = "a".repeat(64);
const THREAD_ID = "b".repeat(64);

test("trimContextsToBudget_underBudget_returnsZeroAndLeavesContextsUnchanged", () => {
  const contexts = { [`msg:${MSG_ID}`]: 100 };
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
  const contexts = {
    [`msg:${MSG_ID}`]: 1,
    [`msg:${"c".repeat(64)}`]: 3,
    [`msg:${"d".repeat(64)}`]: 2,
  };
  const encoder = new TextEncoder();
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
  assert.ok(!(`msg:${MSG_ID}` in contexts));
  const resultSize = encoder.encode(
    JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts }),
  ).length;
  assert.ok(resultSize <= budget);
});

test("trimContextsToBudget_channelKeysNeverEvicted", () => {
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
  assert.ok("channel:some-channel-id" in contexts);
  assert.equal(fitsAfterTrim, true);
  const resultSize = encoder.encode(
    JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts }),
  ).length;
  assert.ok(resultSize <= budget);
});

test("trimContextsToBudget_msgEvictedBeforeThread", () => {
  const contexts = {
    [`msg:${MSG_ID}`]: 1,
    [`thread:${THREAD_ID}`]: 2,
  };
  const encoder = new TextEncoder();
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
  assert.ok(!(`msg:${MSG_ID}` in contexts));
  assert.ok(`thread:${THREAD_ID}` in contexts, "thread entry should survive");
});

test("trimContextsToBudget_emptyContexts_returnsZeroAndFits", () => {
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
  const contexts = {
    "channel:some-channel-id": 100,
  };
  const encoder = new TextEncoder();
  const skeletonSize = encoder.encode(
    JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts }),
  ).length;
  const budget = skeletonSize - 1;
  const { evicted, fitsAfterTrim } = trimContextsToBudget(
    contexts,
    CLIENT_ID,
    budget,
  );
  assert.equal(evicted, 0, "no evictable entries exist");
  assert.equal(fitsAfterTrim, false, "channel-only blob still exceeds budget");
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
  for (const [key] of channelEntries) {
    assert.ok(key in result.slots[0], `${key} should be in slot 0`);
  }
});

test("splitContextsIntoBudgetedSlots_requiresGrowth_allocatesExtraSlot", () => {
  const channelEntries = [];
  for (let i = 0; i < 20; i++) {
    channelEntries.push([makeChannelKey(i), i + 1]);
  }
  const encoder = new TextEncoder();
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
  const allKeys = new Set([
    ...Object.keys(result.slots[0]),
    ...Object.keys(result.slots[1]),
  ]);
  for (const [key] of channelEntries) {
    assert.ok(allKeys.has(key), `${key} should appear in some slot`);
  }
  for (const slotContexts of result.slots) {
    const size = encoder.encode(
      JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts: slotContexts }),
    ).length;
    assert.ok(size <= budget, `slot size ${size} exceeds budget ${budget}`);
  }
});

test("splitContextsIntoBudgetedSlots_exceedsMaxSlots_returnsNull", () => {
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
  assert.ok(makeChannelKey(1) in result.slots[0], "channel key in slot 0");
  assert.ok(makeThreadKey(1) in result.slots[0], "thread key in slot 0");
  assert.ok(makeMsgKey(1) in result.slots[0], "msg key in slot 0");
});

test("splitContextsIntoBudgetedSlots_threadMsgTrimmedWhenPrimarySlotOverBudget", () => {
  const channelEntries = [[makeChannelKey(1), 100]];
  const channelOnlyContexts = { [makeChannelKey(1)]: 100 };
  const channelOnlySize = blobSize(CLIENT_ID, channelOnlyContexts);
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
  assert.ok(makeChannelKey(1) in result.slots[0], "channel key survives");
  assert.ok(!(makeThreadKey(1) in result.slots[0]));
  const size = blobSize(CLIENT_ID, result.slots[0]);
  assert.ok(size <= budget, `slot 0 size ${size} exceeds budget ${budget}`);
});

// ── ReadStateManager.publish — no-op suppression in split mode ────────────────

// Verify that publishSplitSlots returns early (no relay writes) when the
test("publishSplitSlots_noopSuppression_skipsWhenUnchanged", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_filter, _onEvent) =>
      makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };
  const pubkey = "b".repeat(64);
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  const ts = 1_000_000;
  for (let i = 0; i < 700; i++) {
    const channelId = `channel-${i.toString().padStart(64, "0")}`;
    mgr.markContextRead(channelId, ts);
  }
  assert.equal(mgr.currentContexts(), null);
  let publishOneSlotCallCount = 0;
  mgr.publishOneSlot = async (_slotId, contexts) => {
    publishOneSlotCallCount++;
    for (const [key, tsVal] of Object.entries(contexts)) {
      mgr.lastPublishedContexts[key] = tsVal;
    }
  };
  mgr.isLoadComplete = true;
  await mgr.publish();
  const callsAfterFirst = publishOneSlotCallCount;
  assert.ok(callsAfterFirst > 0, "first publish must call publishOneSlot");
  await mgr.publish();
  assert.equal(publishOneSlotCallCount, callsAfterFirst);
  mgr.destroy();
});

// ── NIP-RS override layer: mandatory acceptance tests ─────────────────────────

// Helper: build a ReadStateManager with mocked relay and localStorage.
function makeManager(pubkey = "a".repeat(64)) {
  globalThis.window.localStorage = makeLocalStorage();
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_filter, _onEvent) =>
      makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };
  return new ReadStateManager(pubkey, fakeRelay);
}

// ── Test 1: no ov_* key ever reaches a non-primary slot ──────────────────────
test("splitContextsIntoBudgetedSlots_noOverrideKeyInNonPrimarySlot", () => {
  const ctx1 = "a".repeat(64);
  const ctx2 = "b".repeat(64);
  const ovEntries = [
    [`ov_s:${ctx1}`, 1],
    [`ov_c:${ctx1}`, 0],
    [`ov_b:${ctx1}`, 100],
    [ctx1, 100], // frontier for ctx1 (normal, no escape needed)
    [`ov_s:${ctx2}`, 2],
    [`ov_c:${ctx2}`, 1],
    [`ov_b:${ctx2}`, 200],
    [ctx2, 200], // frontier for ctx2
  ];
  const plainChannelEntries = [];
  for (let i = 0; i < 30; i++) {
    plainChannelEntries.push([makeChannelKey(i), i + 1]);
  }
  const channelEntries = [...ovEntries, ...plainChannelEntries];
  const encoder = new TextEncoder();
  const ovOnlyContexts = Object.fromEntries(ovEntries);
  const ovOnlySize = encoder.encode(
    JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts: ovOnlyContexts }),
  ).length;
  const fifteenPlain = Object.fromEntries(plainChannelEntries.slice(0, 15));
  const fifteenPlainSize = encoder.encode(
    JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts: fifteenPlain }),
  ).length;
  const budget = ovOnlySize + fifteenPlainSize;
  const result = splitContextsIntoBudgetedSlots({
    channelEntries,
    threadMsgEntries: [],
    clientId: CLIENT_ID,
    initialSlotCount: 1,
    maxSlots: 8,
    maxBytes: budget,
    slotIdGenerator: deterministicSlotId,
  });
  assert.ok(result !== null, "should succeed");
  assert.ok(result.slots.length >= 2, "should use at least 2 slots");
  for (let slotIdx = 1; slotIdx < result.slots.length; slotIdx++) {
    const slot = result.slots[slotIdx];
    for (const key of Object.keys(slot)) {
      assert.ok(!key.startsWith("ov_"));
    }
  }
  const slot0 = result.slots[0];
  assert.ok(`ov_s:${ctx1}` in slot0, "ov_s:ctx1 must be in slot 0");
  assert.ok(`ov_c:${ctx1}` in slot0, "ov_c:ctx1 must be in slot 0");
  assert.ok(`ov_b:${ctx1}` in slot0, "ov_b:ctx1 must be in slot 0");
  assert.ok(ctx1 in slot0, "frontier for ctx1 must be in slot 0");
  assert.ok(`ov_s:${ctx2}` in slot0, "ov_s:ctx2 must be in slot 0");
  assert.ok(ctx2 in slot0, "frontier for ctx2 must be in slot 0");
});

// ── Test 1b: reserved esc: raw ID — unescape-before-group rule ───────────────
test("splitContextsIntoBudgetedSlots_escapedFrontierKeyStaysWithItsOverrideGroup", () => {
  const rawCtx = "ov_s:evil";
  const wireKey = `esc:${rawCtx}`; // what currentContexts() emits
  const channelEntries = [
    [`ov_s:${rawCtx}`, 1], // ov_s:ov_s:evil
    [`ov_c:${rawCtx}`, 0], // ov_c:ov_s:evil
    [`ov_b:${rawCtx}`, 50], // ov_b:ov_s:evil
    [wireKey, 50], // esc:ov_s:evil  (escaped frontier)
  ];
  const plain = [];
  for (let i = 0; i < 20; i++) plain.push([makeChannelKey(i), i + 1]);
  const allEntries = [...channelEntries, ...plain];
  const encoder = new TextEncoder();
  const groupOnly = Object.fromEntries(channelEntries);
  const fivePlain = Object.fromEntries(plain.slice(0, 5));
  const budget =
    encoder.encode(
      JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts: groupOnly }),
    ).length +
    encoder.encode(
      JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts: fivePlain }),
    ).length;
  const result = splitContextsIntoBudgetedSlots({
    channelEntries: allEntries,
    threadMsgEntries: [],
    clientId: CLIENT_ID,
    initialSlotCount: 1,
    maxSlots: 8,
    maxBytes: budget,
    slotIdGenerator: deterministicSlotId,
  });
  assert.ok(result !== null, "should succeed");
  assert.ok(result.slots.length >= 2, "should split");
  const slot0 = result.slots[0];
  assert.ok(wireKey in slot0);
  assert.ok(`ov_s:${rawCtx}` in slot0, "ov_s: sibling must be in slot 0");
  assert.ok(`ov_c:${rawCtx}` in slot0, "ov_c: sibling must be in slot 0");
  assert.ok(`ov_b:${rawCtx}` in slot0, "ov_b: sibling must be in slot 0");
  for (let i = 1; i < result.slots.length; i++) {
    for (const key of Object.keys(result.slots[i])) {
      assert.ok(!key.startsWith("ov_") && !key.startsWith("esc:"));
    }
  }
});

// ── Test 2: NIP-RS fenced enumeration — EOSE, lapse, epoch-zero, retry ────────
// All fetchAndMerge tests use subscribeFenced returning a proper FenceHandle so
// the loader only declares complete after an EOSE-established fence.

// Helper to build a minimal valid-looking relay event for the pubkey.
function makeFakeEvent(pubkey, createdAt) {
  return {
    id: `${createdAt.toString(16).padStart(8, "0")}${"0".repeat(56)}`,
    pubkey,
    kind: 30078,
    content: "",
    tags: [],
    created_at: createdAt,
    sig: "s".repeat(128),
  };
}

// ── 2a: empty relay + EOSE → complete ────────────────────────────────────────
test("fetchAndMerge_emptyRelay_setsLoadComplete", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "a".repeat(64);
  let subscribeFencedCallCount = 0;
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_filter, _onEvent) => {
      subscribeFencedCallCount++;
      return makeFenceHandle({ eose: true });
    },
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  await mgr.fetchAndMerge();
  assert.equal(mgr.isLoadComplete, true);
  assert.equal(subscribeFencedCallCount, 1);
  mgr.destroy();
});

// ── 2b/2c: lapse before EOSE (two scenarios: 250ms fallback path, terminal CLOSED) ─
// Both reduce to lapseBeforeEose:true on the fence handle — the loader must
// conclude complete:false in either case.
// biome-ignore format: compact table rows
const lapseBeforeEoseCases = [
  { name: "lapseBeforeEose_setsLoadIncomplete", pubkey: "a1".repeat(32), desc: "lapse before EOSE (e.g. 250 ms fallback path) must produce incomplete load" },
  { name: "terminalClosed_setsLoadIncomplete", pubkey: "a2".repeat(32), desc: "terminal CLOSED (fence lapse before EOSE) must produce incomplete load" },
];
for (const { name, pubkey, desc } of lapseBeforeEoseCases) {
  test(`fetchAndMerge_${name}`, async () => {
    globalThis.window.localStorage = makeLocalStorage();
    const fakeRelay = {
      fetchEvents: async () => [],
      publishEvent: async () => {},
      subscribeToReconnects: () => () => {},
      getConnectionGeneration: () => 0,
      subscribeFenced: async (_filter, _onEvent) =>
        makeFenceHandle({ eose: false, lapseBeforeEose: true }),
    };
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    await mgr.fetchAndMerge();
    assert.equal(mgr.isLoadComplete, false, desc);
    mgr.destroy();
  });
}

// ── 2d: reconnect during post-empty barrier → lapse after tentative complete ──
test("fetchAndMerge_lapseAfterEmptyBand_forcesIncomplete", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "a3".repeat(32);
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_filter, _onEvent) =>
      makeFenceHandle({ eose: true, lapsesAfterEose: true }),
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  await mgr.fetchAndMerge();
  assert.equal(mgr.isLoadComplete, false);
  mgr.destroy();
});

// ── 2e: single event completes after pinned-window discharge ──────────────────
test("fetchAndMerge_singleEvent_completesAfterPinnedWindowDischarge", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "b".repeat(64);
  const event = makeFakeEvent(pubkey, 1000);
  const fakeRelay = {
    fetchEvents: async (filter) => {
      if (filter.since !== undefined && filter.until !== undefined)
        return [event]; // pinned
      if (filter.until !== undefined && filter.until < 1000) return []; // continuation
      return [event]; // initial band
    },
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_filter, _onEvent) =>
      makeFenceHandle({ eose: true }),
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  await mgr.fetchAndMerge();
  assert.equal(mgr.isLoadComplete, true);
  mgr.destroy();
});

// ── 2f: pinned-window-at-cap → incomplete ────────────────────────────────────
test("fetchAndMerge_pinnedWindowAtCap_setsLoadIncomplete", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "c".repeat(64);
  const events = [
    makeFakeEvent(pubkey, 2000),
    makeFakeEvent(pubkey, 2000),
    makeFakeEvent(pubkey, 2000),
  ];
  const fakeRelay = {
    fetchEvents: async (filter) => {
      if (filter.since !== undefined) return events; // pinned window returns cap-many
      return events;
    },
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_filter, _handler) =>
      makeFenceHandle({ eose: true }),
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  await mgr.fetchAndMerge();
  assert.equal(mgr.isLoadComplete, false);
  mgr.destroy();
});

// ── 2g: epoch-zero termination ───────────────────────────────────────────────
test("fetchAndMerge_epochZero_completesAfterPinnedWindowDischarged", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "a4".repeat(32);
  const event = makeFakeEvent(pubkey, 0); // created_at=0
  let continuationCalled = false;
  const fakeRelay = {
    fetchEvents: async (filter) => {
      if (filter.since !== undefined && filter.until !== undefined)
        return [event];
      if (filter.until === 0 && filter.since === undefined) {
        continuationCalled = true;
        return [event]; // a conforming relay returns the event (inclusive filter)
      }
      return [event]; // initial band
    },
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_filter, _onEvent) =>
      makeFenceHandle({ eose: true }),
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  await mgr.fetchAndMerge();
  assert.equal(mgr.isLoadComplete, true);
  assert.equal(continuationCalled, false);
  mgr.destroy();
});

// ── 2h: subscribeFenced throws → fence fails → incomplete ────────────────────
test("fetchAndMerge_fenceFails_setsLoadIncomplete", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "d".repeat(64);
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async () => {
      throw new Error("connection refused");
    },
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  await mgr.fetchAndMerge();
  assert.equal(mgr.isLoadComplete, false);
  mgr.destroy();
});

// ── 2i: incomplete load blocks gated operations ───────────────────────────────
test("fetchAndMerge_incompleteLoad_blocksGatedOperations", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "e".repeat(64);
  const events = [
    makeFakeEvent(pubkey, 1000),
    makeFakeEvent(pubkey, 1000),
    makeFakeEvent(pubkey, 1000),
  ];
  let publishCalls = 0;
  const fakeRelay = {
    fetchEvents: async (filter) => {
      if (filter.since !== undefined) return events; // pinned window
      return events; // band
    },
    publishEvent: async () => {
      publishCalls++;
    },
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.markContextRead("ch", 1000);
  await mgr.fetchAndMerge();
  assert.equal(mgr.isLoadComplete, false, "precondition: load is incomplete");
  mgr.fetchOwnBlobBeforePublish = async () => true;
  await mgr.publish();
  assert.equal(publishCalls, 0, "publish must be blocked when load incomplete");
  const ur = mgr.markChannelUnread("ch");
  assert.equal(ur.status, "queued");
  const rr = mgr.markChannelRead("ch");
  assert.equal(rr.status, "queued");
  mgr.extraSlotIds = ["fakeextraslot0000000000000000000"];
  await mgr.deleteExtraSlots();
  assert.equal(publishCalls, 0);
  mgr.destroy();
});

// ── 2i-b: pre-ready read click → zero register/frontier mutation, zero publish ─
test("preReady_read_witness_b_zeroMutationZeroPublish", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "ee".repeat(32);
  let publishCalls = 0;
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {
      publishCalls++;
    },
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async () => {
      throw new Error("fence timeout");
    },
    subscribeLive: async (_f, _h) => () => {},
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  await mgr.fetchAndMerge();
  assert.equal(mgr.isLoadComplete, false, "precondition: load is incomplete");
  const channelId = "witness-b-ch";
  const regBefore = mgr.overrideRegisters.get(channelId);
  const result = mgr.markChannelRead(channelId);
  assert.equal(result.status, "queued");
  const regAfter = mgr.overrideRegisters.get(channelId);
  assert.equal(regAfter, regBefore);
  assert.equal(regAfter, undefined);
  mgr.fetchOwnBlobBeforePublish = async () => true;
  await mgr.publish();
  assert.equal(publishCalls, 0, "zero publish allowed before complete load");
  mgr.destroy();
});

// ── 2j: production retry path fires on reconnect ─────────────────────────────
test("retryLoad_firesOnReconnect_andClearsIncomplete", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "a5".repeat(32);
  let reconnectCb = null;
  let callRound = 0;
  const fakeRelay = {
    fetchEvents: async (filter) => {
      callRound++;
      if (callRound <= 3) {
        if (filter.since !== undefined)
          return [
            makeFakeEvent(pubkey, 1000),
            makeFakeEvent(pubkey, 1000),
            makeFakeEvent(pubkey, 1000),
          ];
        return [
          makeFakeEvent(pubkey, 1000),
          makeFakeEvent(pubkey, 1000),
          makeFakeEvent(pubkey, 1000),
        ];
      }
      return [];
    },
    publishEvent: async () => {},
    subscribeToReconnects: (cb) => {
      reconnectCb = cb;
      return () => {
        reconnectCb = null;
      };
    },
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  await mgr.initialize();
  assert.equal(mgr.isLoadComplete, false);
  assert.ok(reconnectCb !== null);
  callRound = 999;
  reconnectCb();
  await new Promise((r) => setTimeout(r, 50));
  assert.equal(mgr.isLoadComplete, true);
  mgr.destroy();
});

// ── 2k: direct retry clears incomplete ───────────────────────────────────────
test("fetchAndMerge_retryClears_incomplete", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "f".repeat(64);
  let callRound = 0;
  const fakeRelay = {
    fetchEvents: async (filter) => {
      callRound++;
      if (callRound <= 3) {
        if (filter.since !== undefined)
          return [
            makeFakeEvent(pubkey, 1000),
            makeFakeEvent(pubkey, 1000),
            makeFakeEvent(pubkey, 1000),
          ];
        return [
          makeFakeEvent(pubkey, 1000),
          makeFakeEvent(pubkey, 1000),
          makeFakeEvent(pubkey, 1000),
        ];
      }
      return [];
    },
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  await mgr.fetchAndMerge();
  assert.equal(mgr.isLoadComplete, false, "first load must be incomplete");
  callRound = 999;
  await mgr.fetchAndMerge();
  assert.equal(mgr.isLoadComplete, true);
  mgr.destroy();
});

// ── Test 3: live events go through structured ingest ──────────────────────────
test("handleIncomingEvent_liveOverride_updatesRegisterViaIngest", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "aa".repeat(32);
  const rawCtx = `live-channel-${"x".repeat(51)}`;
  const channelFrontier = 100;
  const blobContexts = {
    [rawCtx]: channelFrontier,
    [`ov_s:${rawCtx}`]: 3,
    [`ov_c:${rawCtx}`]: 1,
    [`ov_b:${rawCtx}`]: 100, // B=100 >= F=100 → active
  };
  const plaintext = JSON.stringify({
    v: 1,
    client_id: "other-device",
    contexts: blobContexts,
  });
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      if (command === "nip44_decrypt_from_self") {
        if (args.ciphertext === "FAKE_CIPHER") return plaintext;
        throw new Error("unknown ciphertext");
      }
      throw new Error(`Unexpected Tauri command: ${command}`);
    },
  };
  let liveHandler = null;
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_filter, _onEvent) =>
      makeFenceHandle({ eose: true }),
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
  };
  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    await mgr.initialize();
    const fakeEvent = {
      id: "b".repeat(64),
      pubkey,
      created_at: 2_000_000,
      kind: 30078,
      tags: [
        ["d", "read-state:a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6"],
        ["t", "read-state"],
      ],
      content: "FAKE_CIPHER",
      sig: "s".repeat(128),
    };
    let decryptCount = 0;
    const origInvoke = globalThis.window.__TAURI_INTERNALS__.invoke;
    globalThis.window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "nip44_decrypt_from_self") decryptCount++;
      return origInvoke(command, args);
    };
    assert.ok(liveHandler !== null, "live subscription must be established");
    liveHandler(fakeEvent); // void-wrapped; wait for async completion
    await new Promise((r) => setTimeout(r, 50));
    assert.equal(decryptCount, 1);
    const reg = mgr.overrideRegisters.get(rawCtx);
    assert.ok(reg, "override register must exist after live delivery");
    assert.equal(reg.s, 3, "S must be 3 from live event");
    assert.equal(reg.c, 1, "C must be 1 from live event");
    assert.equal(reg.b, 100, "B must be 100 from live event");
    const livenessActive = mgr.getOverrideLiveness(rawCtx);
    assert.ok(livenessActive !== null);
    assert.equal(livenessActive.active, true);

    // ── existing-key live clear (higher C defeating S) ───────────────────
    const blobClear = {
      [rawCtx]: channelFrontier,
      [`ov_c:${rawCtx}`]: 4, // tombstone floor: max(S=3,C=1)+1=4
    };
    const ptClear = JSON.stringify({
      v: 1,
      client_id: "other-device-2",
      contexts: blobClear,
    });
    const fakeEventClear = {
      id: "c".repeat(64),
      pubkey,
      created_at: 2_000_001,
      kind: 30078,
      tags: [
        ["d", "read-state:b1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6"],
        ["t", "read-state"],
      ],
      content: "FAKE_CIPHER_2",
      sig: "s".repeat(128),
    };
    globalThis.window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "nip44_decrypt_from_self") {
        if (args.ciphertext === "FAKE_CIPHER_2") return ptClear;
        if (args.ciphertext === "FAKE_CIPHER") return plaintext;
        throw new Error("unknown ciphertext");
      }
      throw new Error(`Unexpected Tauri command: ${command}`);
    };
    liveHandler(fakeEventClear); // void-wrapped; wait for async completion
    await new Promise((r) => setTimeout(r, 50));
    const regAfterClear = mgr.overrideRegisters.get(rawCtx);
    assert.ok(regAfterClear, "register must still exist after clear event");
    assert.equal(regAfterClear.c, 4);
    const liveness = mgr.getOverrideLiveness(rawCtx);
    assert.ok(liveness !== null, "liveness must be available");
    assert.equal(liveness.active, false);
    mgr.destroy();
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
  }
});

// ── Test 4: fetch-before-write failure → zero publishes ──────────────────────
test("publish_fetchOwnBlobFails_doesNotPublish", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "bb".repeat(32);
  let publishCalls = 0;
  const fakeRelay = {
    fetchEvents: async (filter) => {
      if (filter["#d"]) throw new Error("relay unreachable");
      return [];
    },
    publishEvent: async () => {
      publishCalls++;
    },
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.isLoadComplete = true;
  mgr.markContextRead("ch", 1000);
  await mgr.publish();
  assert.equal(publishCalls, 0);
  mgr.destroy();
});

// ── Test 5: durability — register survives restart before debounce ────────────
test("overrideRegister_survivesRestartBeforeDebounce", () => {
  const ls = makeLocalStorage();
  globalThis.window.localStorage = ls;
  const pubkey = "cc".repeat(32);
  const fakeRelay = makeFakeRelay();
  const mgr1 = new ReadStateManager(pubkey, fakeRelay);
  mgr1.isLoadComplete = true;
  mgr1.effectiveState.set("restart-ch", 1000);
  mgr1.publishableContextIds.add("restart-ch");
  const result = mgr1.markChannelUnread("restart-ch");
  assert.equal(result.status, "applied", "mark-unread must succeed");
  mgr1.destroy();
  globalThis.window.localStorage = ls;
  const mgr2 = new ReadStateManager(pubkey, fakeRelay);
  mgr2.hydrateFromLocalStorage();
  const liveness = mgr2.getOverrideLiveness("restart-ch");
  assert.ok(liveness !== null, "register must be hydrated after restart");
  assert.equal(liveness.active, true);
  mgr2.destroy();
});

// ── Test 6: durability — tombstone floor survives restart with fetch failure ──
test("overrideRegister_tombstoneFloorSurvivesRestartWithFetchFailure", async () => {
  const ls = makeLocalStorage();
  globalThis.window.localStorage = ls;
  const pubkey = "dd".repeat(32);
  const goodRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
  };
  const mgr1 = new ReadStateManager(pubkey, goodRelay);
  mgr1.isLoadComplete = true;
  mgr1.effectiveState.set("tombstone-ch", 500);
  mgr1.publishableContextIds.add("tombstone-ch");
  mgr1.markChannelUnread("tombstone-ch"); // S→max(0,0)+1=1, C=0, B→frontier
  mgr1.markChannelRead("tombstone-ch"); // C→max(1,0)+1=2 (clear-wins)
  const tomb = mgr1.overrideRegisters.get("tombstone-ch");
  assert.ok(tomb, "tombstone register must exist");
  assert.equal(tomb.s, 1);
  assert.equal(tomb.c, 2); // max(S=1,C=0)+1 = 2
  mgr1.destroy();
  globalThis.window.localStorage = ls;
  const failRelay = {
    fetchEvents: async () => {
      throw new Error("network unavailable");
    },
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async () => {
      throw new Error("network unavailable");
    },
    subscribeLive: async () => {
      throw new Error("network unavailable");
    },
  };
  const mgr2 = new ReadStateManager(pubkey, failRelay);
  await mgr2.initialize(); // fetch fails, but hydration must have run first
  const reg = mgr2.overrideRegisters.get("tombstone-ch");
  assert.ok(reg, "tombstone register must survive restart with fetch failure");
  assert.equal(reg.s, 1, "S must be preserved");
  assert.equal(reg.c, 2, "C must be preserved (tombstone floor: max(1,0)+1=2)");
  mgr2.destroy();
});

// ── Test 7: budget planner ────────────────────────────────────────────────────
test("markChannelUnread_visibleRefusal_atBudgetExhaustionAndUint32Max", () => {
  const mgr = makeManager();
  mgr.isLoadComplete = true;

  // ── uint32 max refusal ────────────────────────────────────────────────────
  const UINT32_MAX = 0xffffffff;
  const overflowCtx = "overflow-channel";
  mgr.overrideRegisters.set(overflowCtx, {
    s: UINT32_MAX,
    c: UINT32_MAX,
    b: 0,
  });
  mgr.publishableContextIds.add(overflowCtx);
  mgr.effectiveState.set(overflowCtx, 100);
  const overflowResult = mgr.markChannelUnread(overflowCtx);
  assert.equal(overflowResult.status, "refused");
  assert.equal(overflowResult.reason, "uint32_overflow");

  // ── multi-slot success: 700 frontier-only channels → split planner must succeed ─
  const splitMgr = makeManager("f".repeat(64));
  splitMgr.isLoadComplete = true;
  for (let i = 0; i < 700; i++) {
    const ctx = `frontier-ch-${i.toString().padStart(60, "0")}`;
    splitMgr.effectiveState.set(ctx, 1000 + i);
    splitMgr.publishableContextIds.add(ctx);
  }
  const splitCtx = `new-override-ctx-${"n".repeat(48)}`;
  const splitResult = splitMgr.markChannelUnread(splitCtx);
  assert.equal(splitResult.status, "applied");
  assert.ok(splitMgr.overrideRegisters.has(splitCtx));
  splitMgr.destroy();

  // ── near-limit refusal: all 8 slots insufficient → budget_exhausted, no mutation ─
  const fullMgr = makeManager("aa".repeat(32));
  fullMgr.isLoadComplete = true;
  for (let i = 0; i < 250; i++) {
    const ctx = `ov-ch-${i.toString().padStart(60, "0")}`;
    fullMgr.overrideRegisters.set(ctx, { s: 1, c: 0, b: 0 });
    fullMgr.effectiveState.set(ctx, 1000 + i);
    fullMgr.publishableContextIds.add(ctx);
  }
  const nearCtx = `near-limit-ctx-${"z".repeat(49)}`;
  const nearResult = fullMgr.markChannelUnread(nearCtx);
  assert.equal(nearResult.status, "refused");
  assert.equal(nearResult.reason, "budget_exhausted");
  assert.ok(!fullMgr.overrideRegisters.has(nearCtx));
  fullMgr.destroy();
  const reg = mgr.overrideRegisters.get(overflowCtx);
  assert.ok(reg, "overflow channel register must still exist");
  assert.equal(reg.s, UINT32_MAX, "overflow register S must be unchanged");
  mgr.destroy();
});

// ── Test 8: persistence failure — storage_failed + rollback + coherent restart ─
test("markChannelUnread_storageFailure_returnsStorageFailed", () => {
  const throwingLS = makeLocalStorage();
  const originalSetItem = throwingLS.setItem.bind(throwingLS);
  let writeCount = 0;
  throwingLS.setItem = (key, value) => {
    writeCount++;
    if (writeCount > 2) throw new Error("QuotaExceededError");
    originalSetItem(key, value);
  };
  globalThis.window.localStorage = throwingLS;
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager("st".repeat(32), fakeRelay);
  mgr.isLoadComplete = true;
  mgr.effectiveState.set("storage-test-ch", 1000);
  const regBefore = mgr.overrideRegisters.get("storage-test-ch");
  const wasPublishableBefore = mgr.publishableContextIds.has("storage-test-ch");
  const result = mgr.markChannelUnread("storage-test-ch");
  assert.equal(result.status, "refused");
  assert.equal(result.reason, "storage_failed");
  const regAfter = mgr.overrideRegisters.get("storage-test-ch");
  assert.deepEqual(
    regAfter,
    regBefore,
    "overrideRegisters must be rolled back after storage failure",
  );
  assert.equal(
    mgr.publishableContextIds.has("storage-test-ch"),
    wasPublishableBefore,
  );
  const mgr2 = new ReadStateManager("st".repeat(32), fakeRelay);
  mgr2.hydrateFromLocalStorage();
  const regOnRestart = mgr2.overrideRegisters.get("storage-test-ch");
  assert.equal(regOnRestart, undefined);
  mgr.destroy();
  mgr2.destroy();
});

// ── Test 9: inactive existing register still gets C-bump on markChannelRead ───
test("markChannelRead_inactiveExistingRegister_performsCBump", () => {
  const mgr = makeManager();
  mgr.isLoadComplete = true;
  const ctx = "inactive-ch";
  mgr.overrideRegisters.set(ctx, { s: 1, c: 2, b: 0 });
  mgr.publishableContextIds.add(ctx);
  mgr.effectiveState.set(ctx, 100);
  const livenessBefore = mgr.getOverrideLiveness(ctx);
  assert.ok(livenessBefore !== null, "register must exist");
  assert.equal(livenessBefore.active, false, "register must be inactive");
  const result = mgr.markChannelRead(ctx);
  assert.equal(result.status, "applied");
  const reg = mgr.overrideRegisters.get(ctx);
  assert.ok(reg, "register must still exist after markChannelRead");
  assert.equal(reg.c, 3);
  assert.equal(reg.s, 1, "S must be unchanged");
  mgr.destroy();
});

// ── Test 10: coordinate dedupe — newer version wins, older version dropped ────
test("deduplicateByCoordinate_newerVersionWins_olderDropped", async () => {
  const { deduplicateByCoordinate } = await import(
    "./readStateFencedLoader.ts"
  );
  const pubkey = "de".repeat(32);
  const dTag = "read-state:a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6";
  const older = {
    id: "b".repeat(64),
    pubkey,
    created_at: 1_000,
    kind: 30078,
    tags: [["d", dTag]],
    content: "older",
    sig: "s".repeat(128),
  };
  const newer = {
    id: "a".repeat(64), // lower id — for tie-break test below
    pubkey,
    created_at: 2_000,
    kind: 30078,
    tags: [["d", dTag]],
    content: "newer",
    sig: "s".repeat(128),
  };
  const deduped = deduplicateByCoordinate([older, newer]);
  assert.equal(deduped.length, 1, "dedup must yield one event");
  assert.equal(deduped[0].content, "newer", "newer created_at must win");
  const tie1 = { ...older, created_at: 3_000, id: "c".repeat(64) };
  const tie2 = { ...newer, created_at: 3_000, id: "a".repeat(64) };
  const tieDuped = deduplicateByCoordinate([tie1, tie2]);
  assert.equal(tieDuped.length, 1, "tie-break dedup must yield one event");
  assert.equal(tieDuped[0].id, "a".repeat(64), "lower id must win on tie");
});

// ── Test 11: lapse mid-enumeration → incomplete ───────────────────────────────
test("fetchAndMerge_lapseMidEnumeration_setsLoadIncomplete", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "ef".repeat(32);
  const event = makeFakeEvent(pubkey, 1000);
  const fence = makeFenceHandle({ eose: true });
  const fakeRelay = {
    fetchEvents: async (filter) => {
      if (filter.since !== undefined) {
        fence._lapse();
        return [event];
      }
      return [event]; // initial band
    },
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_filter, _onEvent) => fence,
    subscribeLive: async (_f, _h) => () => {},
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  await mgr.fetchAndMerge();
  assert.equal(mgr.isLoadComplete, false);
  mgr.destroy();
});

// ── Test 12: foreign client_id at initial load triggers slot rotation ─────────
test("fetchAndMerge_foreignClientId_rotatesSlotAndUpdatesMetadata", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "a6".repeat(32);
  const slotId = "deadbeefdeadbeef0123456789abcdef"; // will be the initial slotId
  const foreignClientId = "other-client-uuid-9999";
  const blob = JSON.stringify({
    v: 1,
    client_id: foreignClientId,
    contexts: { "ch-conflict": 500 },
  });
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      if (command === "nip44_decrypt_from_self") {
        if (args.ciphertext === "CONFLICT_CIPHER") return blob;
        return JSON.stringify({
          v: 1,
          client_id: foreignClientId,
          contexts: {},
        });
      }
      throw new Error(`Unexpected: ${command}`);
    },
  };
  const conflictEvent = {
    id: "f0".repeat(32),
    pubkey,
    created_at: 12345,
    kind: 30078,
    tags: [
      ["d", `read-state:${slotId}`],
      ["t", "read-state"],
    ],
    content: "CONFLICT_CIPHER",
    sig: "s".repeat(128),
  };
  const fakeRelay = {
    fetchEvents: async (filter) => {
      if (filter.until !== undefined && conflictEvent.created_at > filter.until)
        return [];
      return [conflictEvent];
    },
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
  };
  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    mgr.slotId = slotId;
    await mgr.fetchAndMerge();
    assert.notEqual(mgr.slotId, slotId);
    assert.equal(mgr.maxFetchedCreatedAt, 12345);
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
  }
});

// ── Test 13: frontier-only advance schedules canonical convergence ─────────────
test("ingest_frontierAdvanceFlipsRegister_schedulesCanonicalConvergence", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "a7".repeat(32);
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.isLoadComplete = true;
  const ctx = "convergence-ch";
  mgr.overrideRegisters.set(ctx, { s: 5, c: 0, b: 10 });
  mgr.effectiveState.set(ctx, 5);
  mgr.publishableContextIds.add(ctx);
  const before = mgr.getOverrideLiveness(ctx);
  assert.ok(before?.active, "register must be active before frontier advance");
  const frontierBlob = JSON.stringify({
    v: 1,
    client_id: "peer-device",
    contexts: { [ctx]: 11 }, // frontier advance → F=11 > B=10 → inactive
  });
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke: async (command, _args) => {
      if (command === "nip44_decrypt_from_self") return frontierBlob;
      throw new Error(`Unexpected: ${command}`);
    },
  };
  try {
    const fakeEvent = {
      id: "cc".repeat(32),
      pubkey,
      created_at: 9999,
      kind: 30078,
      tags: [
        ["d", "read-state:cc99aa00112233445566778899aabbcc"],
        ["t", "read-state"],
      ],
      content: "FRONTIER_CIPHER",
      sig: "s".repeat(128),
    };
    await mgr.handleIncomingEvent(fakeEvent);
    const after = mgr.getOverrideLiveness(ctx);
    assert.ok(after !== null, "liveness must still be available");
    assert.equal(after.active, false);
    assert.ok(mgr.debounceTimer !== null);
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
    mgr.destroy();
  }
});

// ── Test 14: createFencedSubscription — EOSE establishes the fence ────────────
test("createFencedSubscription_eoseEstablishes_notLapsed", async () => {
  const subscriptions = new Map();
  let _capturedSubId = null;
  let capturedFencedSub = null;
  const deps = {
    connectionGeneration: () => 0,
    subscriptions,
    sendReq: async (subId) => {
      _capturedSubId = subId;
      capturedFencedSub = subscriptions.get(subId);
    },
    closeSub: async () => {},
    establishmentTimeoutMs: 5_000, // long enough; won't fire in test
  };
  const handle = await createFencedSubscription(
    deps,
    { kinds: [30078], limit: 500 },
    () => {},
  );
  assert.ok(!handle.lapsed, "fence must not be lapsed before EOSE");
  assert.ok(capturedFencedSub, "fenced sub must be registered");
  capturedFencedSub.resolveEstablished();
  await handle.established;
  assert.equal(handle.lapsed, false);
  await handle.unsubscribe();
});

// ── Test 14b: createFencedSubscription — EOSE during sendReq cancels timer ────
test("createFencedSubscription_eoseDuringSend_timerCancelledAndStaysUnlapsed", async () => {
  const subscriptions = new Map();
  const timeoutMs = 12; // short so the test can observe no late lapse
  const deps = {
    connectionGeneration: () => 0,
    subscriptions,
    sendReq: async (subId) => {
      const sub = subscriptions.get(subId);
      if (sub && sub.mode === "fenced") {
        sub.resolveEstablished(); // fires EOSE handler: cancels timer, sets alreadyEstablished
      }
    },
    closeSub: async () => {},
    establishmentTimeoutMs: timeoutMs,
  };
  const handle = await createFencedSubscription(
    deps,
    { kinds: [30078], limit: 500 },
    () => {},
  );
  await handle.established;
  assert.equal(handle.lapsed, false);
  await new Promise((r) => setTimeout(r, timeoutMs * 3));
  assert.equal(handle.lapsed, false);
  await handle.unsubscribe();
});

// ── Test 15: createFencedSubscription — CLOSED lapses before EOSE ────────────
test("createFencedSubscription_closedBeforeEose_lapses", async () => {
  const subscriptions = new Map();
  const deps = {
    connectionGeneration: () => 0,
    subscriptions,
    sendReq: async (subId) => {
      const sub = subscriptions.get(subId);
      if (sub && sub.mode === "fenced") {
        sub.lapsed = true;
        sub.resolveEstablished();
        subscriptions.delete(subId);
      }
    },
    closeSub: async () => {},
    establishmentTimeoutMs: 5_000,
  };
  const handle = await createFencedSubscription(
    deps,
    { kinds: [30078], limit: 500 },
    () => {},
  );
  await handle.established;
  assert.equal(handle.lapsed, true, "CLOSED before EOSE must set lapsed=true");
});

// ── Test 16: createFencedSubscription — resetConnection lapses all fences ─────
test("createFencedSubscription_resetConnection_lapses", async () => {
  const subscriptions = new Map();
  const deps = {
    connectionGeneration: () => 0,
    subscriptions,
    sendReq: async () => {}, // REQ sent; EOSE never arrives
    closeSub: async () => {},
    establishmentTimeoutMs: 5_000,
  };
  const handle = await createFencedSubscription(
    deps,
    { kinds: [30078], limit: 500 },
    () => {},
  );
  assert.equal(handle.lapsed, false, "fence must not be lapsed before reset");
  for (const [subId, sub] of subscriptions) {
    if (sub.mode === "fenced") {
      sub.lapsed = true;
      sub.resolveEstablished();
      subscriptions.delete(subId);
    }
  }
  await handle.established;
  assert.equal(handle.lapsed, true);
});

// ── Test 17: createFencedSubscription — establishment timeout lapses fence ────
test("createFencedSubscription_establishmentTimeout_lapses", async () => {
  const subscriptions = new Map();
  const deps = {
    connectionGeneration: () => 0,
    subscriptions,
    sendReq: async () => {}, // REQ sent; EOSE never arrives
    closeSub: async () => {},
    establishmentTimeoutMs: 10, // fast for test
  };
  const handle = await createFencedSubscription(
    deps,
    { kinds: [30078], limit: 500 },
    () => {},
  );
  assert.equal(handle.lapsed, false);
  await handle.established;
  assert.equal(handle.lapsed, true);
  assert.equal(subscriptions.size, 0);
});

// ── Test 18: createFencedSubscription — timeout does NOT count as establishment
test("createFencedSubscription_timeout_doesNotCountAsEstablishment", async () => {
  const subscriptions = new Map();
  const deps = {
    connectionGeneration: () => 0,
    subscriptions,
    sendReq: async () => {},
    closeSub: async () => {},
    establishmentTimeoutMs: 10,
  };
  const handle = await createFencedSubscription(
    deps,
    { kinds: [30078], limit: 500 },
    () => {},
  );
  await handle.established; // timeout fires here
  assert.equal(handle.lapsed, true);
});

// ── Test 19: establishment timeout → no-EOSE relay produces incomplete load ───
test("fetchAndMerge_establishmentTimeout_setsLoadIncomplete", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "b0".repeat(32);
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (filter, onEvent) => {
      const subscriptions = new Map();
      const deps = {
        connectionGeneration: () => 0,
        subscriptions,
        sendReq: async () => {}, // REQ sent; EOSE never arrives
        closeSub: async () => {},
        establishmentTimeoutMs: 20, // fast for test
      };
      return createFencedSubscription(deps, filter, onEvent);
    },
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  const raceMs = 500; // generous but bounded
  const result = await Promise.race([
    mgr.fetchAndMerge().then(() => "settled"),
    new Promise((r) => setTimeout(() => r("timeout"), raceMs)),
  ]);
  assert.equal(result, "settled");
  assert.equal(mgr.isLoadComplete, false);
  mgr.destroy();
});

// ── Test 20: early-write-fails / v2-succeeds → outcome + restart agree ────────
test("markChannelUnread_earlyWriteFails_v2Succeeds_outcomeAndRestartAgree", () => {
  const throwingLS = makeLocalStorage();
  const originalSetItem = throwingLS.setItem.bind(throwingLS);
  let writeCount = 0;
  let blockAncillary = false;
  throwingLS.setItem = (key, value) => {
    writeCount++;
    if (blockAncillary && writeCount > 3) throw new Error("QuotaExceededError");
    originalSetItem(key, value);
  };
  globalThis.window.localStorage = throwingLS;
  const fakeRelay = makeFakeRelay();
  const pubkey = "ea".repeat(32);
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.isLoadComplete = true;
  mgr.effectiveState.set("committed-ch", 1000);
  blockAncillary = true;
  const result = mgr.markChannelUnread("committed-ch");
  blockAncillary = false;
  assert.equal(result.status, "applied");
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const stored = throwingLS.getItem(v2Key);
  assert.ok(stored !== null, "v2 key must be present after committed action");
  const parsed = JSON.parse(stored);
  assert.ok(parsed.r !== undefined && "committed-ch" in parsed.r);
  writeCount = 0; // reset counter so reconstruction writes go through
  const mgr2 = new ReadStateManager(pubkey, fakeRelay);
  mgr2.hydrateFromLocalStorage(); // simulate the init-time hydration step
  mgr2.isLoadComplete = true;
  assert.ok(mgr2.publishableContextIds.has("committed-ch"));
  const contexts2 = mgr2.currentContexts();
  assert.ok(contexts2 !== null);
  const hasEntry = Object.keys(contexts2 ?? {}).some(
    (k) => k.startsWith("committed-ch") || k.includes("committed-ch"),
  );
  assert.ok(hasEntry);
  mgr.destroy();
  mgr2.destroy();
});

// ── Test 21: v2-write-fails → storage_failed, slot IDs unchanged ─────────────
test("markChannelUnread_v2WriteFails_slotIdsUnchanged", () => {
  const throwingLS = makeLocalStorage();
  const originalSetItem = throwingLS.setItem.bind(throwingLS);
  let blockV2 = false;
  throwingLS.setItem = (key, value) => {
    if (blockV2 && key.startsWith("buzz.nip-rs.override-state.v2:")) {
      throw new Error("QuotaExceededError");
    }
    originalSetItem(key, value);
  };
  globalThis.window.localStorage = throwingLS;
  const fakeRelay = makeFakeRelay();
  const pubkey = "eb".repeat(32);
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.isLoadComplete = true;
  for (let i = 0; i < 700; i++) {
    const ctx = `channel-id-${String(i).padStart(5, "0")}-${"x".repeat(30)}`;
    mgr.effectiveState.set(ctx, 1_700_000_000 + i);
    mgr.publishableContextIds.add(ctx);
  }
  mgr.effectiveState.set("slot-test-ch", 1000);
  const prevExtraSlotIdsMem = mgr.extraSlotIds.slice();
  const extraSlotKey = `buzz.nip-rs.extra-slot-ids:${pubkey}`;
  const prevExtraSlotIdsStored = throwingLS.getItem(extraSlotKey);
  blockV2 = true;
  const result = mgr.markChannelUnread("slot-test-ch");
  blockV2 = false;
  assert.equal(result.status, "refused");
  assert.equal(result.reason, "storage_failed");
  assert.deepEqual(
    mgr.extraSlotIds,
    prevExtraSlotIdsMem,
    "extraSlotIds in memory must be rolled back on v2 write failure",
  );
  const afterExtraSlotIdsStored = throwingLS.getItem(extraSlotKey);
  assert.equal(afterExtraSlotIdsStored, prevExtraSlotIdsStored);
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const v2Stored = throwingLS.getItem(v2Key);
  const v2Parsed = v2Stored ? JSON.parse(v2Stored) : {};
  assert.equal("slot-test-ch" in v2Parsed, false);
  mgr.destroy();
});

// ── Test 22b: stale ancillary frontier suppressed by authoritative v2 frontier ─
test("hydrateFromLocalStorage_staleAncillaryFrontier_v2FrontierWins", () => {
  const ls = makeLocalStorage();
  globalThis.window.localStorage = ls;
  const pubkey = "ec".repeat(32);
  const ctx = "stale-frontier-ch";
  const staleFrontierKey = `buzz.channel-read-state.v2:${pubkey}`;
  ls.setItem(
    staleFrontierKey,
    JSON.stringify({ [ctx]: new Date(50 * 1_000).toISOString() }),
  );
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  ls.setItem(v2Key, JSON.stringify({ [ctx]: { s: 5, c: 0, b: 100, f: 101 } }));
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  assert.equal(mgr.effectiveState.get(ctx), 101);
  mgr.isLoadComplete = true;
  const contexts = mgr.currentContexts();
  assert.ok(contexts !== null, "currentContexts must not be null");
  const ovSKey = `ov_s:${ctx}`;
  const ovCKey = `ov_c:${ctx}`;
  assert.equal(ovSKey in contexts, false);
  assert.ok(ovCKey in contexts);
  mgr.destroy();
});

// ── Test 23: Amendment A — same-source read→unread reincarnation ──────────────
test("drain_sameSourceReincarnation_genFenceZeroLocalEffects", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "d1".repeat(32);
  const channelId = `reincarnation-channel-${"a".repeat(40)}`;
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);
  mgr.overrideRegisters.set(channelId, { s: 1, c: 0, b: 50 });
  mgr.publishableContextIds.add(channelId);
  assert.equal(mgr.isLoadComplete, false);
  const readResult = mgr.markChannelRead(channelId);
  assert.equal(readResult.status, "queued");
  const unreadResult = mgr.markChannelUnread(channelId);
  assert.equal(unreadResult.status, "queued");
  const currentIntent = pendingOverrideIntentStore.get(channelId);
  assert.ok(currentIntent, "intent must exist after re-mark");
  assert.equal(currentIntent.op, "unread");
  assert.equal(currentIntent.gen, 2, "current intent must be gen=2");
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const reg = mgr.overrideRegisters.get(channelId);
  assert.ok(reg, "override register must exist after drain");
  assert.ok(reg.s > 1, "S must be bumped by the unread drain (gen=2 applied)");
  assert.ok(reg.c === 0);
  assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  mgr.destroy();
});

// ── Test 24: Amendment B — drain crash window 1 ───────────────────────────────
test("drain_crashWindow1_restartReplaysOnce_extraBumpOnly", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "d2".repeat(32);
  const channelId = `crash-window-1-ch-${"b".repeat(44)}`;
  const fakeRelay = makeFakeRelay();

  // ── Session 1: enqueue, then "crash" before drain can commit ──────────────
  const mgr1 = new ReadStateManager(pubkey, fakeRelay);
  mgr1.effectiveState.set(channelId, 100);
  assert.equal(mgr1.isLoadComplete, false, "precondition: load incomplete");
  const q1 = mgr1.markChannelUnread(channelId);
  assert.equal(q1.status, "queued");
  mgr1.destroy();

  // ── Session 2: restart. Intent present, no receipt → replay fires once ────
  const mgr2 = new ReadStateManager(pubkey, fakeRelay);
  mgr2.hydrateFromLocalStorage();
  mgr2.effectiveState.set(channelId, 100);
  mgr2.isLoadComplete = true;
  const intentAfterRestart = pendingOverrideIntentStore.get(channelId);
  assert.ok(intentAfterRestart);
  assert.equal(intentAfterRestart.op, "unread");
  assert.equal(mgr2.appliedReceipts.get(channelId), undefined);
  await mgr2.drainPendingIntents(mgr2.loadGeneration);
  const reg = mgr2.overrideRegisters.get(channelId);
  assert.ok(reg, "register must exist after drain-on-restart");
  assert.ok(reg.s > 0, "S must be bumped (replay fired once)");
  assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  mgr2.destroy();
});

// ── Test 25: Amendment B — drain crash window 2 ───────────────────────────────
test("drain_crashWindow2_receiptPresent_restartNoRebump", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "d3".repeat(32);
  const channelId = `crash-window-2-ch-${"c".repeat(44)}`;
  const ls = globalThis.window.localStorage;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const sSingleCommit = 2;
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: { [channelId]: { s: sSingleCommit, c: 0, b: 100, f: 100 } },
      receipts: { [channelId]: { intentGen: 1, op: "unread" } },
      pi: { [channelId]: { gen: 1, op: "unread" } },
      ng: 2,
    }),
  );
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  mgr.effectiveState.set(channelId, 100);
  mgr.isLoadComplete = true;
  const receipt = mgr.appliedReceipts.get(channelId);
  assert.ok(receipt);
  assert.equal(receipt.intentGen, 1, "receipt intentGen must be 1");
  assert.equal(receipt.op, "unread", "receipt op must be unread");
  const intent = pendingOverrideIntentStore.get(channelId);
  assert.ok(intent, "intent must be present (simulated crash before delete)");
  assert.equal(intent.gen, 1, "intent gen must be 1");
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const regAfterRestart = mgr.overrideRegisters.get(channelId);
  assert.ok(regAfterRestart, "register must still exist after restart drain");
  assert.equal(regAfterRestart.s, sSingleCommit);
  assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  mgr.destroy();
});

// ── Tests 26/27: Amendment C crash races (mirror pair) ───────────────────────
// biome-ignore format: compact table rows
const crashAfterAppliedCases = [
  {
    name: "drain_crashAfterAppliedUnread_newerRemoteRead_noRebump",
    // Unread applied (receipt durable); remote publishes C=6 (read).
    // Receipt prevents second S-bump that would reverse the phone's read.
    pubkey: "d4".repeat(32), channelId: `amend-c-unread-race-${"d".repeat(44)}`,
    initReg: { s: 5, c: 3, b: 100, f: 100 }, receiptOp: "unread",
    remoteCtx: (ch) => ({ [`ov_s:${ch}`]: 5, [`ov_c:${ch}`]: 6, [`ov_b:${ch}`]: 100, [ch]: 200 }),
    remoteSlot: "d4".repeat(16), blobId: "d4".repeat(32),
    assertDrain(reg) { assert.equal(reg.s, 5, "S must NOT be re-bumped"); assert.ok(reg.c >= 6, "C must reflect remote read"); },
  },
  {
    name: "drain_crashAfterAppliedRead_newerRemoteUnread_noRebump",
    // Read applied (C-bump + receipt durable); remote publishes S=7 (unread).
    // Receipt prevents second C-bump that would reverse the new unread.
    pubkey: "d5".repeat(32), channelId: `amend-c-read-race-${"e".repeat(45)}`,
    initReg: { s: 5, c: 6, b: 100, f: 200 }, receiptOp: "read",
    remoteCtx: (ch) => ({ [`ov_s:${ch}`]: 7, [`ov_c:${ch}`]: 6, [`ov_b:${ch}`]: 100, [ch]: 100 }),
    remoteSlot: "d5".repeat(16), blobId: "d5".repeat(32),
    assertDrain(reg) { assert.equal(reg.c, 6, "C must NOT be re-bumped"); assert.ok(reg.s >= 7, "remote S=7 must be preserved"); },
  },
];
for (const row of crashAfterAppliedCases) {
  test(row.name, async () => {
    const { pubkey, channelId } = row;
    const initBlob = {
      r: { [channelId]: row.initReg },
      receipts: { [channelId]: { intentGen: 1, op: row.receiptOp } },
      pi: { [channelId]: { gen: 1, op: row.receiptOp } },
      ng: 2,
    };
    const remoteBlob = JSON.stringify({
      v: 1,
      client_id: "remote-device-client",
      contexts: row.remoteCtx(channelId),
    });
    const blobEvent = {
      id: row.blobId,
      pubkey,
      created_at: 9999,
      kind: 30078,
      tags: [
        ["d", `read-state:${row.remoteSlot}`],
        ["t", "read-state"],
      ],
      content: "CIPHER",
      sig: "s".repeat(128),
    };
    let fetchCallCount = 0;
    globalThis.window.__TAURI_INTERNALS__ = {
      invoke: async (cmd) => {
        if (cmd === "nip44_decrypt_from_self") return remoteBlob;
        throw new Error(`Unexpected: ${cmd}`);
      },
    };
    const fakeRelay = {
      fetchEvents: async () => {
        fetchCallCount++;
        return fetchCallCount === 1 ? [blobEvent] : [];
      },
      publishEvent: async () => {},
      subscribeFenced: async (_f, handler) => {
        if (fetchCallCount === 0) handler(blobEvent);
        return makeFenceHandle({ eose: true });
      },
      subscribeLive: async (_f, _h) => () => {},
      subscribeToReconnects: () => () => {},
      getConnectionGeneration: () => 0,
    };
    try {
      globalThis.window.localStorage = makeLocalStorage();
      const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
      globalThis.window.localStorage.setItem(v2Key, JSON.stringify(initBlob));
      const mgr = new ReadStateManager(pubkey, fakeRelay);
      mgr.hydrateFromLocalStorage();
      const receipt = mgr.appliedReceipts.get(channelId);
      assert.ok(receipt, "receipt must be loaded on restart");
      assert.equal(receipt.intentGen, 1);
      assert.equal(receipt.op, row.receiptOp);
      await mgr.fetchAndMerge(mgr.loadGeneration);
      assert.equal(mgr.isLoadComplete, true, "load must complete");
      const regAfterLoad = mgr.overrideRegisters.get(channelId);
      assert.ok(regAfterLoad, "register must exist after fetchAndMerge");
      await mgr.drainPendingIntents(mgr.loadGeneration);
      const reg = mgr.overrideRegisters.get(channelId);
      assert.ok(reg, "register must exist after drain");
      row.assertDrain(reg);
      assert.equal(
        pendingOverrideIntentStore.get(channelId),
        undefined,
        "intent must be deleted after alreadyApplied drain",
      );
      mgr.destroy();
    } finally {
      delete globalThis.window.__TAURI_INTERNALS__;
    }
  });
}

// ── Test 28: Amendment C — receipt without matching intent is swept silently ──
test("drain_receiptWithoutMatchingIntent_sweptWithoutRegisterEffect", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "d6".repeat(32);
  const channelId = `receipt-sweep-ch-${"f".repeat(46)}`;
  const ls = globalThis.window.localStorage;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: { [channelId]: { s: 3, c: 0, b: 100, f: 100 } },
      receipts: { [channelId]: { intentGen: 1, op: "unread" } },
    }),
  );
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  mgr.isLoadComplete = true;
  const receipt = mgr.appliedReceipts.get(channelId);
  assert.equal(receipt, undefined);
  const intent = pendingOverrideIntentStore.get(channelId);
  assert.equal(intent, undefined);
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const reg = mgr.overrideRegisters.get(channelId);
  assert.ok(reg, "register must still exist");
  assert.equal(reg.s, 3);
  const stored = JSON.parse(ls.getItem(v2Key) ?? "{}");
  const receiptsInBlob = stored.receipts ?? {};
  assert.equal(Object.keys(receiptsInBlob).length, 0);
  mgr.destroy();
});

// ── Test 29: Amendment C — register+receipt atomicity (no torn state) ─────────
test("drain_registerReceiptAtomicity_noStateWhereOnePersistsWithoutOther", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "d7".repeat(32);
  const channelId = `atomicity-ch-${"a".repeat(51)}`;
  const ls = globalThis.window.localStorage;
  let blockV2Writes = false;
  const originalSetItem = ls.setItem.bind(ls);
  ls.setItem = (key, value) => {
    if (blockV2Writes && key.startsWith("buzz.nip-rs.override-state.v2:")) {
      throw new Error("QuotaExceededError");
    }
    originalSetItem(key, value);
  };
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 100);
  mgr.isLoadComplete = false;
  const q = mgr.markChannelUnread(channelId);
  assert.equal(q.status, "queued");
  mgr.isLoadComplete = true;
  blockV2Writes = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  blockV2Writes = false;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const stored = JSON.parse(ls.getItem(v2Key) ?? "null");
  const registersInBlob = stored?.r ?? {};
  assert.equal(Object.keys(registersInBlob).length, 0);
  const receiptsInBlob = stored?.receipts ?? {};
  assert.equal(Object.keys(receiptsInBlob).length, 0);
  const intent = pendingOverrideIntentStore.get(channelId);
  assert.ok(intent, "intent must survive when drain's v2 write fails");
  assert.equal(intent.op, "unread");
  mgr.destroy();
});

// ── Test 22: fetchOwnBlobBeforePublish processes foreign client_id ────────────
test("fetchOwnBlobBeforePublish_foreignClientId_rotatesSlotAndUpdatesMetadata", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "b1".repeat(32);
  const slotId = "aabbccddeeff00112233445566778899";
  const foreignClientId = "foreign-client-read-before-write";
  const blob = JSON.stringify({
    v: 1,
    client_id: foreignClientId,
    contexts: {},
  });
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke: async (command, _args) => {
      if (command === "nip44_decrypt_from_self") return blob;
      throw new Error(`Unexpected: ${command}`);
    },
  };
  const blobEvent = {
    id: "f1".repeat(32),
    pubkey,
    created_at: 55555,
    kind: 30078,
    tags: [
      ["d", `read-state:${slotId}`],
      ["t", "read-state"],
    ],
    content: "BLOB_CIPHER",
    sig: "s".repeat(128),
  };
  const fakeRelay = {
    fetchEvents: async () => [blobEvent],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
  };
  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    mgr.slotId = slotId;
    mgr.isLoadComplete = true;
    await mgr.fetchOwnBlobBeforePublish();
    assert.notEqual(mgr.slotId, slotId);
    assert.equal(mgr.maxFetchedCreatedAt, 55555);
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
  }
});

// ── Test 30: Amendment A structural — gen2 enqueued DURING replay is durably committed ──
test("drain_gen2EnqueuedDuringReplay_bufferedUntilTransactionCommit", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "30".repeat(32);
  const channelId = `amend-a-biting-ch-${"b".repeat(46)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);
  mgr.isLoadComplete = false;
  const q = mgr.markChannelUnread(channelId);
  assert.equal(q.status, "queued", "gen1 must be queued");
  const gen1 = pendingOverrideIntentStore.get(channelId).gen;
  assert.ok(gen1 >= 1, "gen1 must be a positive generation number");
  const origTry = mgr.tryCandidatePlan.bind(mgr);
  let injected = false;
  mgr.tryCandidatePlan = (id, reg) => {
    const ok = origTry(id, reg);
    if (!injected && id === channelId) {
      injected = true;
      pendingOverrideIntentStore.enqueue(channelId, "unread");
    }
    return ok;
  };
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  await new Promise((r) => setTimeout(r, 10));
  const reg = mgr.overrideRegisters.get(channelId);
  assert.ok(reg !== undefined);
  assert.ok(reg.s > 0, "gen1 S-bump must be present");
  const rawBlob = globalThis.window.localStorage.getItem(v2Key);
  const blob = rawBlob ? JSON.parse(rawBlob) : {};
  assert.ok(blob.r?.[channelId], "gen1 register must be persisted in v2 blob");
  assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  mgr.destroy();
});

// ── Test 31: unread→read inversion — read gen replaces unread gen ─────────────
test("drain_unreadToReadInversion_onlyReadApplied", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "31".repeat(32);
  const channelId = `inversion-ur-ch-${"c".repeat(48)}`;
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);
  mgr.overrideRegisters.set(channelId, { s: 1, c: 0, b: 50 });
  mgr.publishableContextIds.add(channelId);
  mgr.isLoadComplete = false;
  const r1 = mgr.markChannelUnread(channelId);
  assert.equal(r1.status, "queued");
  const gen1 = pendingOverrideIntentStore.get(channelId).gen;
  const r2 = mgr.markChannelRead(channelId);
  assert.equal(r2.status, "queued");
  const intent = pendingOverrideIntentStore.get(channelId);
  assert.equal(intent.op, "read", "current intent must be read (gen2)");
  assert.ok(intent.gen > gen1, "gen2 must exceed gen1");
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const reg = mgr.overrideRegisters.get(channelId);
  assert.ok(reg, "register must exist after read drain");
  assert.equal(reg.s, 1);
  assert.ok(reg.c > 0, "C must be bumped by the read drain (gen2)");
  assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  mgr.destroy();
});

// ── Test 32: read→unread inversion — unread gen replaces read gen ─────────────
test("drain_readToUnreadInversion_onlySBumpApplied", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "32".repeat(32);
  const channelId = `inversion-ru-ch-${"d".repeat(48)}`;
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);
  mgr.isLoadComplete = false;
  const r1 = mgr.markChannelRead(channelId);
  assert.equal(r1.status, "queued");
  const r2 = mgr.markChannelUnread(channelId);
  assert.equal(r2.status, "queued");
  const intent = pendingOverrideIntentStore.get(channelId);
  assert.equal(intent.op, "unread", "current intent must be unread (gen2)");
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const reg = mgr.overrideRegisters.get(channelId);
  assert.ok(reg, "register must exist after unread drain");
  assert.ok(reg.s > 0, "S must be bumped by unread drain (gen2)");
  assert.equal(reg.c, 0);
  assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  mgr.destroy();
});

// ── Test 33: non-reentrant drain transaction — deferred enqueue durably committed ─
test("drain_compareDeletePreservesNewerIntent", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "33".repeat(32);
  const channelId = `cmp-del-newer-ch-${"e".repeat(47)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const forcedKey = `buzz-forced-unread.v1:${pubkey}`;
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);
  mgr.isLoadComplete = false;
  const initialForcedEntry = { markerAtWhenForced: 50, sources: ["manual"] };
  globalThis.window.localStorage.setItem(
    forcedKey,
    JSON.stringify({ [channelId]: initialForcedEntry }),
  );
  mgr.markChannelUnread(channelId);
  const gen1 = pendingOverrideIntentStore.get(channelId).gen;
  const cb = wireDrainCallback(mgr, pubkey);
  const origCallback = mgr.onDrainOutcome;
  let gen2PromotedAfterCallback = false;
  let injectedOnce = false;
  mgr.onDrainOutcome = (outcome) => {
    origCallback(outcome);
    if (
      !injectedOnce &&
      outcome.kind === "applied-unread" &&
      outcome.channelId === channelId
    ) {
      injectedOnce = true;
      pendingOverrideIntentStore.enqueue(channelId, "unread");
      gen2PromotedAfterCallback = true;
    }
  };
  let capturedDrainFn = null;
  const origScheduleDrain = mgr.scheduleDrain.bind(mgr);
  mgr.scheduleDrain = () => {
    capturedDrainFn = () => {
      origScheduleDrain();
    };
  };
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // ── Intermediate assertions (before gen2 auto-drain) ──────────────────────
  assert.ok(gen2PromotedAfterCallback);
  let rawBlob = globalThis.window.localStorage.getItem(v2Key);
  let blob = rawBlob ? JSON.parse(rawBlob) : {};
  assert.ok(blob.r?.[channelId]);
  assert.ok(blob.pi?.[channelId]);
  assert.equal(blob.receipts?.[channelId], undefined);
  assert.ok(blob.pi[channelId].gen > gen1, "gen2 gen in blob must exceed gen1");
  {
    const mgr2 = new ReadStateManager(pubkey, fakeRelay);
    mgr2.hydrateFromLocalStorage();
    const hydratedGen2 = pendingOverrideIntentStore.get(channelId);
    assert.ok(hydratedGen2);
    assert.equal(hydratedGen2.op, "unread", "gen2 op must be unread");
    assert.ok(hydratedGen2.gen > gen1);
    mgr2.destroy();
  }
  assert.equal(cb.versionBumps, 0);

  // ── Fire the gen2 drain ────────────────────────────────────────────────────
  assert.ok(capturedDrainFn, "scheduleDrain must have been called");
  mgr.scheduleDrain = origScheduleDrain;
  capturedDrainFn();
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  rawBlob = globalThis.window.localStorage.getItem(v2Key);
  blob = rawBlob ? JSON.parse(rawBlob) : {};
  assert.equal(blob.pi?.[channelId], undefined);
  cb.teardown();
  mgr.destroy();
});

// ── Test 34: Amendment C via production load — restart + fetchAndMerge ────────
test("drain_amendmentC_restart_fetchAndMerge_noBumpAfterReceipt", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "34".repeat(32);
  const channelId = `amend-c-prod-ch-${"f".repeat(48)}`;
  const ls = globalThis.window.localStorage;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: { [channelId]: { s: 5, c: 3, b: 100, f: 100 } },
      receipts: { [channelId]: { intentGen: 1, op: "unread" } },
      pi: { [channelId]: { gen: 1, op: "unread" } },
      ng: 2,
    }),
  );
  const remoteBlob = JSON.stringify({
    v: 1,
    client_id: "remote-device-client",
    contexts: {
      [`ov_s:${channelId}`]: 5,
      [`ov_c:${channelId}`]: 6,
      [`ov_b:${channelId}`]: 100,
      [channelId]: 200, // frontier advanced past B → override inactive
    },
  });
  const remoteSlotId = "0d".repeat(16);
  const blobEvent = {
    id: "ab".repeat(32),
    pubkey,
    created_at: 9999,
    kind: 30078,
    tags: [
      ["d", `read-state:${remoteSlotId}`],
      ["t", "read-state"],
    ],
    content: "CIPHER",
    sig: "s".repeat(128),
  };
  let fetchCallCount = 0;
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      if (command === "nip44_decrypt_from_self") return remoteBlob;
      throw new Error(`Unexpected: ${command}`);
    },
  };
  const fakeRelay = {
    fetchEvents: async () => {
      fetchCallCount++;
      return fetchCallCount === 1 ? [blobEvent] : [];
    },
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_filter, handler) => {
      if (fetchCallCount === 0) {
        handler(blobEvent);
      }
      return makeFenceHandle({ eose: true });
    },
    subscribeLive: async (_f, _h) => () => {},
  };
  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    mgr.hydrateFromLocalStorage();
    const receipt = mgr.appliedReceipts.get(channelId);
    assert.ok(receipt, "receipt must be loaded on restart");
    assert.equal(receipt.intentGen, 1);
    await mgr.fetchAndMerge(mgr.loadGeneration);
    assert.equal(mgr.isLoadComplete, true, "load must complete");
    await mgr.drainPendingIntents(mgr.loadGeneration);
    const reg = mgr.overrideRegisters.get(channelId);
    assert.ok(reg, "register must exist");
    assert.equal(reg.s, 5);
    assert.ok(reg.c >= 6, "C must reflect remote C=6 merge");
    assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
  }
});

// ── Test 35: atomic deletion — intent + receipt deleted in one persist ─────────
test("drain_cleanupCommit_intentAndReceiptDeletedAtomically", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "35".repeat(32);
  const channelId = `atomic-del-ch-${"g".repeat(50)}`;
  const ls = globalThis.window.localStorage;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);
  mgr.isLoadComplete = false;
  mgr.markChannelUnread(channelId);
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const rawAfter = ls.getItem(v2Key);
  const blobAfter = rawAfter ? JSON.parse(rawAfter) : {};
  const piEntry = blobAfter?.pi?.[channelId];
  assert.equal(piEntry, undefined);
  const receiptEntry = blobAfter?.receipts?.[channelId];
  assert.equal(receiptEntry, undefined);
  const regEntry = blobAfter?.r?.[channelId];
  assert.ok(regEntry, "register must persist after successful drain");
  mgr.destroy();
});

// ── Tests 36/36b: incomplete verdict schedules automatic retry (mirror pair) ──
// biome-ignore format: compact table rows
const incompleteVerdictCases = [
  // retryLoad(): attempt-0 is the call itself (immediate, no delay), so after
  // the first incomplete verdict retryAttempt is 1. The baseline used `> 0`.
  { name: "retryLoad_incompleteVerdictSchedulesAutomaticRetry",   pubkey: "36".repeat(32),              trigger: (mgr) => mgr.retryLoad(),   expectedAttempt: 1 },
  // initialize(): attempt-0 is also consumed by the call, same invariant.
  // The baseline required the exact value 1; that tighter assertion is restored here.
  { name: "initialize_incompleteVerdict_schedulesAutomaticRetry", pubkey: "36b".repeat(21).slice(0,64), trigger: (mgr) => mgr.initialize(),  expectedAttempt: 1 },
];
for (const row of incompleteVerdictCases) {
  test(row.name, async () => {
    let fetchCallCount = 0;
    const timers = makeFakeTimers();
    try {
      globalThis.window.localStorage = makeLocalStorage();
      const fakeRelay = makeFakeRelay({
        subscribeFenced: async () => {
          fetchCallCount++;
          throw new Error("fence timeout");
        },
      });
      const mgr = new ReadStateManager(row.pubkey, fakeRelay);
      assert.equal(mgr.retryAttempt, 0, "precondition: retryAttempt is 0");
      await row.trigger(mgr);
      assert.equal(fetchCallCount, 1, "first fetch must have been called");
      assert.equal(
        mgr.retryAttempt,
        row.expectedAttempt,
        `retryAttempt must be ${row.expectedAttempt} after first incomplete`,
      );
      assert.ok(
        mgr.retryBackoffTimer !== null,
        "retryBackoffTimer must be set",
      );
      assert.ok(timers.scheduled.size > 0, "a timer must be pending");
      const [timerId, { fn: timerFn, ms }] = [...timers.scheduled.entries()][0];
      assert.ok(ms > 0, "backoff timer delay must be > 0");
      timers.scheduled.delete(timerId);
      const genBeforeFire = mgr.loadGeneration;
      timerFn();
      await new Promise((r) => timers.origSetTimeout(r, 50));
      assert.ok(
        fetchCallCount >= 2,
        "automatic retry must trigger a second fetch",
      );
      assert.ok(
        mgr.loadGeneration >= genBeforeFire,
        "load generation must not regress",
      );
      assert.ok(
        mgr.retryBackoffTimer !== null || timers.scheduled.size > 0,
        "follow-up timer must be scheduled",
      );
      mgr.destroy();
      assert.equal(
        mgr.retryBackoffTimer,
        null,
        "destroy must clear retryBackoffTimer",
      );
      assert.equal(timers.scheduled.size, 0, "no pending timers after destroy");
    } finally {
      timers.restore();
    }
  });
}

// ── Test 37: controller — reconnect coalescing, backoff cancel, fresh gen/fetch ──
test("retryLoad_reconnectDuringLoadInFlight_coalescesToPendingRetry", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "37".repeat(32);
  let resolveFirst;
  const firstBarrier = new Promise((r) => {
    resolveFirst = r;
  });
  let resolveSecond;
  const secondBarrier = new Promise((r) => {
    resolveSecond = r;
  });
  const origSetTimeout = globalThis.window.setTimeout;
  const origClearTimeout = globalThis.window.clearTimeout;
  const scheduledTimers = new Map(); // id → { fn, ms } — only ACTIVE (not yet cleared) timers
  const allScheduledTimerIds = new Set(); // ALL timer IDs ever created (including cleared)
  const clearedTimerIds = new Set();
  let nextFakeId = 37000;
  globalThis.window.setTimeout = (fn, ms) => {
    if (ms > 0) {
      const id = nextFakeId++;
      scheduledTimers.set(id, { fn, ms });
      allScheduledTimerIds.add(id);
      return id;
    }
    return origSetTimeout(fn, ms);
  };
  globalThis.window.clearTimeout = (id) => {
    if (scheduledTimers.has(id)) {
      clearedTimerIds.add(id);
      scheduledTimers.delete(id);
    } else {
      origClearTimeout(id);
    }
  };
  let fetchCallCount = 0;
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_f, _h) => {
      const call = ++fetchCallCount;
      if (call === 1) {
        await firstBarrier;
        throw new Error("fence timeout"); // incomplete
      }
      if (call === 2) {
        await secondBarrier;
        throw new Error("fence timeout"); // incomplete
      }
      throw new Error("fence timeout");
    },
    subscribeLive: async (_f, _h) => () => {},
  };
  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);

    // ── Phase 1: start first load, block it at the barrier ────────────────────
    const load1Promise = mgr.retryLoad();
    await new Promise((r) => origSetTimeout(r, 0)); // let async fn reach await
    assert.equal(mgr.loadInFlight, true);
    const gen1 = mgr.loadGeneration;

    // ── Phase 2: first reconnect while load is in-flight ──────────────────────
    mgr.retryLoad();
    assert.equal(mgr.pendingRetryOnComplete, true);
    assert.equal(mgr.loadGeneration, gen1);
    mgr.retryLoad();
    assert.equal(mgr.pendingRetryOnComplete, true);
    assert.equal(mgr.loadGeneration, gen1);

    // ── Phase 3: release gen-1 fetch — incomplete verdict fires ───────────────
    resolveFirst();
    await load1Promise;
    assert.ok(allScheduledTimerIds.size >= 1);
    const firstBackoffTimerId = [...allScheduledTimerIds][0];

    // ── Phase 4: finally-block fires pending retry ─────────────────────────────
    await new Promise((r) => origSetTimeout(r, 0));
    assert.ok(clearedTimerIds.has(firstBackoffTimerId));
    assert.equal(scheduledTimers.size, 0);
    assert.equal(mgr.loadGeneration, gen1 + 1);
    assert.equal(fetchCallCount, 2, "exactly two fetches must have occurred");

    // ── Phase 5: gen-2 fetch is in-flight — multiple reconnects coalesce ─────────
    assert.equal(mgr.loadInFlight, true);
    mgr.retryLoad();
    assert.equal(mgr.pendingRetryOnComplete, true);
    mgr.retryLoad();
    assert.equal(mgr.pendingRetryOnComplete, true);
    assert.equal(mgr.loadGeneration, gen1 + 1);

    // ── Phase 6: release gen-2 fetch — pending retry fires gen-3 load ─────────
    resolveSecond();
    await new Promise((r) => origSetTimeout(r, 0));
    assert.equal(mgr.loadGeneration, gen1 + 2);
    assert.equal(fetchCallCount, 3, "exactly three fetches must have occurred");
    mgr.destroy();
    assert.equal(mgr.retryBackoffTimer, null);
    assert.equal(scheduledTimers.size, 0);
  } finally {
    globalThis.window.setTimeout = origSetTimeout;
    globalThis.window.clearTimeout = origClearTimeout;
  }
});

// ── Test 38: controller — stale-completion ordering via gen check ──────────────
test("fetchAndMerge_staleGeneration_doesNotSetLoadComplete", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "38".repeat(32);
  let resolveFirst;
  const firstBarrier = new Promise((r) => {
    resolveFirst = r;
  });
  let callCount = 0;
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_f, _h) => {
      callCount++;
      if (callCount === 1) {
        await firstBarrier;
        return makeFenceHandle({ eose: true });
      }
      return makeFenceHandle({ eose: true });
    },
    subscribeLive: async (_f, _h) => () => {},
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  const gen1 = mgr.loadGeneration;
  const firstFetch = mgr.fetchAndMerge(gen1);
  await new Promise((r) => setTimeout(r, 0));
  ++mgr.loadGeneration;
  const gen2 = mgr.loadGeneration;
  await mgr.fetchAndMerge(gen2);
  assert.equal(mgr.isLoadComplete, true, "gen2 load must complete");
  resolveFirst();
  await firstFetch;
  assert.equal(mgr.isLoadComplete, true);
  mgr.destroy();
});

// ── Test 39: controller — destroy cancels backoff timer ───────────────────────
test("retryLoad_destroyWhileTimerPending_cancelsTimer", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "39".repeat(32);
  let timerSet = false;
  let pendingTimerResolve = null; // captured so we can drain after destroy
  const origSetTimeout = globalThis.window.setTimeout;
  const origClearTimeout = globalThis.window.clearTimeout;
  const activeTimerIds = new Set();
  globalThis.window.setTimeout = (fn, ms) => {
    if (ms > 0) {
      timerSet = true;
      const id = origSetTimeout(() => {
        activeTimerIds.delete(id);
        pendingTimerResolve = null;
        fn();
      }, 60_000); // long enough never to fire during test
      pendingTimerResolve = fn;
      activeTimerIds.add(id);
      return id;
    }
    return origSetTimeout(fn, ms);
  };
  globalThis.window.clearTimeout = (id) => {
    activeTimerIds.delete(id);
    origClearTimeout(id);
  };
  try {
    const fakeRelay = {
      fetchEvents: async () => [],
      publishEvent: async () => {},
      subscribeToReconnects: () => () => {},
      getConnectionGeneration: () => 0,
      subscribeFenced: async () => {
        throw new Error("lapse");
      },
      subscribeLive: async (_f, _h) => () => {},
    };
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    await mgr.retryLoad();
    const retryPromise = mgr.retryLoad();
    await new Promise((r) => origSetTimeout(r, 0));
    assert.equal(timerSet, true, "a backoff timer must have been scheduled");
    assert.ok(mgr.retryBackoffTimer !== null);
    mgr.destroy();
    assert.equal(mgr.retryBackoffTimer, null);
    assert.equal(activeTimerIds.size, 0);
    if (pendingTimerResolve) pendingTimerResolve();
    await retryPromise;
  } finally {
    globalThis.window.setTimeout = origSetTimeout;
    globalThis.window.clearTimeout = origClearTimeout;
  }
});

// ── Test 40: controller — exactly one drain per complete-load generation ───────
test("retryLoad_exactlyOneDrainPerCompleteGeneration", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "40".repeat(32);
  const channelId = `one-drain-ch-${"h".repeat(51)}`;
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.isLoadComplete = false;
  mgr.markChannelUnread(channelId);
  await mgr.retryLoad();
  assert.equal(mgr.isLoadComplete, true, "load must be complete");
  const regAfterFirst = mgr.overrideRegisters.get(channelId);
  const sAfterFirst = regAfterFirst?.s ?? 0;
  await mgr.retryLoad();
  const regAfterSecond = mgr.overrideRegisters.get(channelId);
  assert.equal(regAfterSecond?.s ?? 0, sAfterFirst);
  mgr.destroy();
});

// ── Test 41: store/UI — identity swap during drain cancels stale gen ──────────
test("drain_identitySwapDuringDrain_staleGenCancelled", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkeyA = "41".repeat(32);
  const channelId = `identity-swap-ch-${"i".repeat(47)}`;
  const fakeRelay = makeFakeRelay();
  const mgrA = new ReadStateManager(pubkeyA, fakeRelay);
  mgrA.effectiveState.set(channelId, 50);
  mgrA.isLoadComplete = false;
  mgrA.markChannelUnread(channelId);
  const intentA = pendingOverrideIntentStore.get(channelId);
  assert.ok(intentA, "intent must exist for pubkeyA");
  mgrA.destroy();
  await mgrA.drainPendingIntents(mgrA.loadGeneration);
  const intentAfter = pendingOverrideIntentStore.get(channelId);
  assert.ok(intentAfter, "intent must survive drain when manager is destroyed");
  pendingOverrideIntentStore.compareAndDelete(channelId, intentAfter.gen);
});

// ── Test 42: deferred refusal — queued unread intent refused by drain ─────────
test("drain_queuedUnread_refused_surfacesOutcome", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "42".repeat(32);
  const channelId = `deferred-refusal-ch-${"j".repeat(44)}`;
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);
  mgr.isLoadComplete = false;
  mgr.overrideRegisters.set(channelId, { s: 0xffffffff, c: 0, b: 50 });
  mgr.publishableContextIds.add(channelId);
  const q = mgr.markChannelUnread(channelId);
  assert.equal(q.status, "queued", "must queue pre-ready");
  const outcomes = [];
  mgr.onDrainOutcome = (outcome) => {
    outcomes.push(outcome);
  };
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  assert.equal(outcomes.length, 1, "exactly one outcome must be emitted");
  assert.equal(outcomes[0].channelId, channelId);
  assert.equal(outcomes[0].kind, "genuine-refusal");
  assert.equal(outcomes[0].op, "unread");
  assert.equal(outcomes[0].reason, "uint32_overflow");
  assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  mgr.destroy();
});

// ── Test 43: deferred-refusal-after-restart — intent persisted, hydrated, refused ─
test("drain_deferredRefusalAfterRestart_intentHydratedAndRefused", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "43".repeat(32);
  const channelId = `deferred-restart-ch-${"k".repeat(44)}`;
  const ls = globalThis.window.localStorage;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: { [channelId]: { s: 0xffffffff, c: 0, b: 50, f: 50 } },
      pi: { [channelId]: { gen: 1, op: "unread" } },
      ng: 2,
    }),
  );
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  const hydrated = pendingOverrideIntentStore.get(channelId);
  assert.ok(hydrated, "intent must be hydrated from v2 blob on restart");
  assert.equal(hydrated.op, "unread");
  assert.equal(hydrated.gen, 1);
  const outcomes = [];
  mgr.onDrainOutcome = (outcome) => {
    outcomes.push(outcome);
  };
  await mgr.fetchAndMerge(mgr.loadGeneration);
  assert.equal(mgr.isLoadComplete, true, "load must complete");
  await mgr.drainPendingIntents(mgr.loadGeneration);
  assert.equal(outcomes.length, 1, "exactly one outcome must be emitted");
  assert.equal(outcomes[0].kind, "genuine-refusal");
  assert.equal(outcomes[0].op, "unread");
  assert.equal(outcomes[0].reason, "uint32_overflow");
  assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  mgr.destroy();
});

// ── Test 44: storage_failed — enqueue survives; in-memory intent is live ─────
test("markChannelUnread_storageFailed_intentInMemoryOnly", () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "44".repeat(32);
  const channelId = `storage-fail-ch-${"l".repeat(50)}`;
  const origSetItem = globalThis.window.localStorage.setItem;
  globalThis.window.localStorage.setItem = () => {
    throw new Error("QuotaExceededError");
  };
  try {
    const fakeRelay = {
      fetchEvents: async () => [],
      publishEvent: async () => {},
      subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
      subscribeLive: async (_f, _h) => () => {},
      subscribeToReconnects: () => () => {},
      getConnectionGeneration: () => 0,
    };
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    mgr.isLoadComplete = false;
    const result = mgr.markChannelUnread(channelId);
    assert.equal(result.status, "queued");
    const intent = pendingOverrideIntentStore.get(channelId);
    assert.ok(intent, "intent must be in-memory even if persist failed");
    assert.equal(intent.op, "unread");
    globalThis.window.localStorage.setItem = origSetItem;
    mgr.isLoadComplete = true;
    return mgr.drainPendingIntents(mgr.loadGeneration).then(() => {
      const reg = mgr.overrideRegisters.get(channelId);
      assert.ok(reg, "register must exist after drain");
      assert.ok(reg.s > 0, "S-bump must have been applied by drain");
      assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
      mgr.destroy();
    });
  } finally {
    globalThis.window.localStorage.setItem = origSetItem;
  }
});

// ── Test 45: authoritative queued-read target captured at click time ──────────
test("markChannelRead_preReady_capturesAuthoritativeMarkAt", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "45".repeat(32);
  const channelId = `mark-at-ch-${"m".repeat(53)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const forcedKey = `buzz-forced-unread.v1:${pubkey}`;
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  const partialFrontier = 100;
  mgr.effectiveState.set(channelId, partialFrontier);
  mgr.isLoadComplete = false;
  mgr.overrideRegisters.set(channelId, { s: 5, c: 0, b: 100 });
  mgr.publishableContextIds.add(channelId);
  const forcedEntry = { markerAtWhenForced: 50, sources: ["inbox", "manual"] };
  globalThis.window.localStorage.setItem(
    forcedKey,
    JSON.stringify({ [channelId]: forcedEntry }),
  );
  const authoritativeMarkAt = 999;
  const sourceScope = "inbox";
  const r = mgr.markChannelRead(channelId, sourceScope, authoritativeMarkAt);
  assert.equal(r.status, "queued", "must queue when load incomplete");
  const intent = pendingOverrideIntentStore.get(channelId);
  assert.ok(intent, "intent must exist");
  assert.equal(intent.readTarget, authoritativeMarkAt);
  const cb = wireDrainCallback(mgr, pubkey);
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const frontier = mgr.effectiveState.get(channelId) ?? 0;
  assert.ok(frontier >= authoritativeMarkAt);
  const rawBlob = globalThis.window.localStorage.getItem(v2Key);
  const blob = rawBlob ? JSON.parse(rawBlob) : {};
  assert.ok(blob.r?.[channelId]);
  const reg = blob.r[channelId];
  assert.ok(reg.c > 0, "C must be bumped (applied read)");
  assert.equal(blob.pi?.[channelId], undefined);
  assert.equal(blob.receipts?.[channelId], undefined);
  const storedForced = forcedUnreadStore.read(pubkey);
  const afterEntry = storedForced[channelId];
  assert.ok(
    afterEntry !== undefined,
    'channel entry must still exist (only "inbox" removed, "manual" remains)',
  );
  const sources =
    typeof afterEntry === "object" && afterEntry !== null
      ? afterEntry.sources
      : ["manual"];
  assert.ok(
    !sources.includes("inbox"),
    '"inbox" source must be removed after applied-read drain',
  );
  assert.ok(
    sources.includes("manual"),
    '"manual" source must remain after "inbox"-only cleanup',
  );
  assert.equal(cb.versionBumps, 1);
  cb.teardown();
  mgr.destroy();
});

// ── Test 46: already_inactive — source cleanup runs via hook/store ────────────
test("drain_alreadyInactive_sourceCleanupCallback", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "46".repeat(32);
  const channelId = `already-inactive-ch-${"n".repeat(44)}`;
  const forcedKey = `buzz-forced-unread.v1:${pubkey}`;
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 100);
  mgr.isLoadComplete = false;
  const forcedEntry = {
    markerAtWhenForced: 100,
    sources: ["inbox", "manual"],
  };
  globalThis.window.localStorage.setItem(
    forcedKey,
    JSON.stringify({ [channelId]: forcedEntry }),
  );
  const sourceScope = "inbox";
  const r = mgr.markChannelRead(channelId, sourceScope);
  assert.equal(r.status, "queued");
  const cb = wireDrainCallback(mgr, pubkey);
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const storedForced = forcedUnreadStore.read(pubkey);
  const afterEntry = storedForced[channelId];
  assert.ok(
    afterEntry !== undefined,
    'entry must still exist after "inbox"-only removal (manual remains)',
  );
  const sources =
    typeof afterEntry === "object" && afterEntry !== null
      ? afterEntry.sources
      : ["manual"];
  assert.ok(
    !sources.includes("inbox"),
    '"inbox" source must be removed by silent-inactive hook',
  );
  assert.ok(
    sources.includes("manual"),
    '"manual" source must remain unchanged',
  );
  assert.equal(cb.versionBumps, 1);
  assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  cb.teardown();
  mgr.destroy();
});

// ── Test 47: restart-safe unread rollback — priorForcedEntry restored in forcedUnreadStore
test("drain_unreadRefusal_restartSafe_priorForcedEntryRestored", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "47".repeat(32);
  const channelId = `restart-safe-ch-${"o".repeat(47)}`;
  const ls = globalThis.window.localStorage;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const forcedKey = `buzz-forced-unread.v1:${pubkey}`;
  const priorForcedEntry = { markerAtWhenForced: 50, sources: ["inbox"] };
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: { [channelId]: { s: 0xffffffff, c: 0, b: 50, f: 50 } },
      pi: {
        [channelId]: {
          gen: 1,
          op: "unread",
          priorForcedEntry: priorForcedEntry,
        },
      },
      ng: 2,
    }),
  );
  ls.setItem(
    forcedKey,
    JSON.stringify({
      [channelId]: { markerAtWhenForced: 50, sources: ["manual"] },
    }),
  );
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  const hydrated = pendingOverrideIntentStore.get(channelId);
  assert.ok(hydrated, "intent must be hydrated");
  assert.deepEqual(
    hydrated.priorForcedEntry,
    priorForcedEntry,
    "priorForcedEntry must survive round-trip through v2 blob",
  );
  const cb = wireDrainCallback(mgr, pubkey);
  await mgr.fetchAndMerge(mgr.loadGeneration);
  assert.equal(mgr.isLoadComplete, true);
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const storedForced = forcedUnreadStore.read(pubkey);
  assert.deepEqual(
    storedForced[channelId],
    priorForcedEntry,
    "forcedUnreadStore must be restored to priorForcedEntry after post-restart refusal",
  );
  assert.equal(cb.versionBumps, 1);
  cb.teardown();
  mgr.destroy();
});

// ── Test 48: legacy null priorForcedEntry — hydrates correctly, drain runs ────
test("hydrate_legacyNullPriorForcedEntry_intentHydratedAndDrains", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "48".repeat(32);
  const channelId1 = `legacy-null-ch-${"p".repeat(49)}`;
  const channelId2 = `sibling-null-ch-${"q".repeat(48)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: {
        [channelId1]: { s: 0xffffffff, c: 0, b: 50, f: 50 },
        [channelId2]: { s: 5, c: 3, b: 50, f: 50 },
      },
      pi: {
        [channelId1]: { gen: 1, op: "unread", priorForcedEntry: null },
        [channelId2]: { gen: 2, op: "unread" },
      },
      ng: 3,
    }),
  );
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  const intent1 = pendingOverrideIntentStore.get(channelId1);
  assert.ok(intent1, "ch1 intent (null prior) must hydrate");
  assert.equal(intent1.gen, 1, "ch1 intent gen must be 1");
  assert.equal("priorForcedEntry" in intent1, true);
  assert.equal(intent1.priorForcedEntry, null);
  const intent2 = pendingOverrideIntentStore.get(channelId2);
  assert.ok(intent2);
  assert.equal(intent2.gen, 2, "ch2 intent gen must be 2");
  mgr.effectiveState.set(channelId1, 0); // frontier below b=50
  mgr.effectiveState.set(channelId2, 10);
  await mgr.fetchAndMerge(mgr.loadGeneration);
  assert.equal(mgr.isLoadComplete, true);
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const reg2 = mgr.overrideRegisters.get(channelId2);
  assert.ok(reg2, "ch2 register must be present after drain");
  assert.equal(reg2.s, 6, "ch2 S must be bumped by drain");
  assert.equal(pendingOverrideIntentStore.get(channelId1), undefined);
  assert.equal(pendingOverrideIntentStore.get(channelId2), undefined);
  mgr.destroy();
});

// ── Test 49: corrupt v2 blob fields — malformed intents rejected, valid sibling hydrates ──
test("hydrate_corruptV2BlobIntentFields_rejectedWithoutBlockingValidSibling", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "49".repeat(32);
  const channelId1 = `corrupt-rt-ch-${"r".repeat(50)}`;
  const channelId2 = `corrupt-pfe-ch-${"s".repeat(48)}`;
  const channelId3 = `corrupt-src-ch-${"t".repeat(48)}`;
  const channelId4 = `valid-sibling-${"u".repeat(50)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: {
        [channelId4]: { s: 3, c: 1, b: 80, f: 80 },
      },
      pi: {
        [channelId1]: { gen: 1, op: "read", readTarget: 1e300 },
        [channelId2]: {
          gen: 2,
          op: "unread",
          priorForcedEntry: { markerAtWhenForced: -1, sources: ["inbox"] },
        },
        [channelId3]: {
          gen: 3,
          op: "unread",
          priorForcedEntry: { sources: ["unknown-source"] },
        },
        [channelId4]: { gen: 4, op: "unread" },
      },
      ng: 5,
    }),
  );
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  assert.doesNotThrow(
    () => mgr.hydrateFromLocalStorage(),
    "hydrateFromLocalStorage must not throw on corrupt intent fields",
  );
  assert.equal(pendingOverrideIntentStore.get(channelId1), undefined);
  assert.equal(pendingOverrideIntentStore.get(channelId2), undefined);
  assert.equal(pendingOverrideIntentStore.get(channelId3), undefined);
  const intent4 = pendingOverrideIntentStore.get(channelId4);
  assert.ok(intent4);
  assert.equal(intent4.gen, 4, "ch4 intent gen must be 4");
  assert.equal(intent4.op, "unread", "ch4 intent op must be unread");
  mgr.effectiveState.set(channelId4, 50);
  await mgr.fetchAndMerge(mgr.loadGeneration);
  assert.equal(mgr.isLoadComplete, true);
  let drainThrew = false;
  try {
    await mgr.drainPendingIntents(mgr.loadGeneration);
  } catch {
    drainThrew = true;
  }
  assert.equal(drainThrew, false);
  assert.equal(pendingOverrideIntentStore.get(channelId4), undefined);
  mgr.destroy();
});

// ── Test 50: overflow with frontier-deactivated register → silent success ─────
test("markChannelReadDirect_uint32overflow_frontierDeactivated_silentSuccess", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "50".repeat(32);
  const channelId = `overflow-deact-ch-${"v".repeat(46)}`;
  const forcedKey = `buzz-forced-unread.v1:${pubkey}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50); // initial frontier
  mgr.isLoadComplete = false;
  mgr.overrideRegisters.set(channelId, { s: 0xffffffff, c: 0, b: 50 });
  mgr.publishableContextIds.add(channelId);
  const forcedEntry = { markerAtWhenForced: 50, sources: ["manual"] };
  ls.setItem(forcedKey, JSON.stringify({ [channelId]: forcedEntry }));
  const sourceScope = "manual";
  const r = mgr.markChannelRead(channelId, sourceScope, 100);
  assert.equal(r.status, "queued", "must queue when load incomplete");
  const cb = wireDrainCallback(mgr, pubkey);
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const frontier = mgr.effectiveState.get(channelId) ?? 0;
  assert.ok(frontier >= 100);
  const storedForced = forcedUnreadStore.read(pubkey);
  assert.equal(storedForced[channelId], undefined);
  assert.equal(cb.versionBumps, 1);
  assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  const rawBlob = ls.getItem(v2Key);
  const blob = rawBlob ? JSON.parse(rawBlob) : {};
  assert.equal(blob.pi?.[channelId], undefined);
  cb.teardown();
  mgr.destroy();
});

// ── Test 51: abort/retry on step-1 storage failure — gen1 survives, retry drains ──
test("drain_step1StorageFailure_abortRetry_gen1SurvivesAndRetrains", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "51".repeat(32);
  const channelId = `abort-step1-ch-${"w".repeat(49)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;
  const timers = makeFakeTimers();
  let blockV2Writes = false;
  const origSetItem = ls.setItem.bind(ls);
  ls.setItem = (key, value) => {
    if (blockV2Writes && key.startsWith("buzz.nip-rs.override-state.v2:")) {
      throw new Error("QuotaExceededError");
    }
    origSetItem(key, value);
  };
  const fakeRelay = makeFakeRelay();
  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    mgr.effectiveState.set(channelId, 50);
    mgr.isLoadComplete = false;
    mgr.markChannelUnread(channelId);
    const gen1 = pendingOverrideIntentStore.get(channelId).gen;
    const blobBefore = JSON.parse(ls.getItem(v2Key) ?? "null") ?? {};
    assert.equal(blobBefore.pi?.[channelId]?.gen, gen1);
    mgr.onDrainOutcome = (outcome) => {
      if (
        outcome.kind === "applied-unread" &&
        outcome.channelId === channelId
      ) {
        const prev = mgr.isLoadComplete;
        mgr.isLoadComplete = false; // route through deferred latch path
        mgr.markChannelRead(channelId, undefined, 999);
        mgr.isLoadComplete = prev;
      }
    };
    mgr.isLoadComplete = true;
    blockV2Writes = true;
    await mgr.drainPendingIntents(mgr.loadGeneration);
    blockV2Writes = false;
    assert.equal(mgr.overrideRegisters.get(channelId), undefined);
    const blobAfterFail = JSON.parse(ls.getItem(v2Key) ?? "null") ?? {};
    assert.equal(blobAfterFail.pi?.[channelId]?.gen, gen1);
    assert.equal(blobAfterFail.r?.[channelId], undefined);
    const intentAfterFail = pendingOverrideIntentStore.get(channelId);
    assert.ok(intentAfterFail, "gen1 must be live after abort");
    assert.equal(intentAfterFail.gen, gen1);
    assert.equal(timers.scheduled.size, 1);
    assert.ok(mgr.drainRetryTimer !== null);
    assert.ok(timers.scheduled.has(mgr.drainRetryTimer));
    {
      const mgr2 = new ReadStateManager(pubkey, fakeRelay);
      mgr2.hydrateFromLocalStorage();
      const hydratedIntent = pendingOverrideIntentStore.get(channelId);
      assert.ok(hydratedIntent);
      assert.equal(hydratedIntent.gen, gen1, "hydrated intent must be gen1");
      mgr2.destroy();
    }
    mgr.hydrateFromLocalStorage();
    const drainRetryTimerId = mgr.drainRetryTimer;
    const retryFn = timers.scheduled.get(drainRetryTimerId)?.fn;
    assert.ok(retryFn, "abort retry timer fn must be in timers.scheduled");
    timers.scheduled.clear(); // clear all before firing
    await retryFn(); // fires the drain retry
    await new Promise((r) => timers.origSetTimeout(r, 0));
    const regAfterRetry = mgr.overrideRegisters.get(channelId);
    assert.ok(regAfterRetry, "register must be committed after retry");
    assert.ok(regAfterRetry.s > 0, "S must be bumped (gen1 unread applied)");
    const blobAfterRetry = JSON.parse(ls.getItem(v2Key) ?? "null") ?? {};
    assert.equal(blobAfterRetry.receipts?.[channelId], undefined);
    const frontierAfterRetry = mgr.effectiveState.get(channelId) ?? 0;
    assert.ok(frontierAfterRetry >= 999);
    assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
    {
      const mgr3 = new ReadStateManager(pubkey, fakeRelay);
      mgr3.hydrateFromLocalStorage();
      assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
      const f3 = mgr3.effectiveState.get(channelId) ?? 0;
      assert.ok(f3 >= 999);
      mgr3.destroy();
    }
    mgr.destroy();
  } finally {
    timers.restore();
  }
});

// ── Test 52: abort/retry on step-3 cleanup failure — gen1 stays live, retry cleans up ──
test("drain_step3CleanupFailure_abortRetry_gen1StaysLiveAndRetrains", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "52".repeat(32);
  const channelId = `abort-step3-ch-${"x".repeat(49)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;
  const timers = makeFakeTimers();
  let v2WriteCount = 0;
  let blockFromWriteIndex = -1;
  const origSetItem = ls.setItem.bind(ls);
  ls.setItem = (key, value) => {
    if (key.startsWith("buzz.nip-rs.override-state.v2:")) {
      if (blockFromWriteIndex >= 0 && v2WriteCount >= blockFromWriteIndex) {
        throw new Error("QuotaExceededError");
      }
      v2WriteCount++;
    }
    origSetItem(key, value);
  };
  const fakeRelay = makeFakeRelay();
  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    mgr.effectiveState.set(channelId, 50);
    mgr.isLoadComplete = false;
    mgr.markChannelUnread(channelId);
    const gen1 = pendingOverrideIntentStore.get(channelId).gen;
    mgr.onDrainOutcome = (outcome) => {
      if (
        outcome.kind === "applied-unread" &&
        outcome.channelId === channelId
      ) {
        const prev = mgr.isLoadComplete;
        mgr.isLoadComplete = false; // route through deferred latch path
        mgr.markChannelRead(channelId, undefined, 999);
        mgr.isLoadComplete = prev;
      }
    };
    blockFromWriteIndex = 2;
    mgr.isLoadComplete = true;
    await mgr.drainPendingIntents(mgr.loadGeneration);
    blockFromWriteIndex = -1;
    const reg = mgr.overrideRegisters.get(channelId);
    assert.ok(reg, "register must be committed after step-1 succeeds");
    assert.ok(reg.s > 0, "S must be bumped");
    const blobAfterFail = JSON.parse(ls.getItem(v2Key) ?? "null") ?? {};
    assert.ok(blobAfterFail.r?.[channelId]);
    assert.ok(blobAfterFail.pi?.[channelId]);
    assert.ok(blobAfterFail.receipts?.[channelId]);
    const intentAfterFail = pendingOverrideIntentStore.get(channelId);
    assert.ok(intentAfterFail, "gen1 must be live after step-3 abort");
    assert.equal(intentAfterFail.gen, gen1, "live intent must be gen1");
    const drainRetryTimerId = mgr.drainRetryTimer;
    assert.ok(drainRetryTimerId !== null);
    assert.ok(timers.scheduled.has(drainRetryTimerId));
    {
      const mgr2 = new ReadStateManager(pubkey, fakeRelay);
      mgr2.hydrateFromLocalStorage();
      const hydratedIntent = pendingOverrideIntentStore.get(channelId);
      assert.ok(hydratedIntent, "gen1 must hydrate from blob");
      assert.equal(hydratedIntent.gen, gen1, "hydrated must be gen1");
      mgr2.destroy();
    }
    mgr.hydrateFromLocalStorage();
    const retryFn = timers.scheduled.get(drainRetryTimerId)?.fn;
    assert.ok(retryFn, "retry timer fn must still be pending");
    timers.scheduled.clear(); // clear all (publish debounce + retry) before firing
    await retryFn();
    await new Promise((r) => timers.origSetTimeout(r, 0));
    const blobAfterRetry = JSON.parse(ls.getItem(v2Key) ?? "null") ?? {};
    assert.equal(blobAfterRetry.pi?.[channelId], undefined);
    assert.equal(blobAfterRetry.receipts?.[channelId], undefined);
    const frontierAfterRetry = mgr.effectiveState.get(channelId) ?? 0;
    assert.ok(frontierAfterRetry >= 999);
    assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
    {
      const mgr3 = new ReadStateManager(pubkey, fakeRelay);
      mgr3.hydrateFromLocalStorage();
      assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
      const f3 = mgr3.effectiveState.get(channelId) ?? 0;
      assert.ok(f3 >= 999);
      mgr3.destroy();
    }
    mgr.destroy();
  } finally {
    timers.restore();
  }
});

// ── Test 53: ng normalization — gen7 collision cannot discard a fresh action ──
test("hydrate_ngCollision_normalizedAboveExistingGens_freshActionNotDiscarded", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "53".repeat(32);
  const channelId = `ng-collision-ch-${"y".repeat(47)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: {
        [channelId]: { s: 5, c: 3, b: 100, f: 100 },
      },
      receipts: {
        [channelId]: { intentGen: 7, op: "read" },
      },
      pi: {
        [channelId]: { gen: 7, op: "read", readTarget: 50 },
      },
      ng: 7,
    }),
  );
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  const hydratedIntent = pendingOverrideIntentStore.get(channelId);
  assert.ok(hydratedIntent, "intent (gen=7) must hydrate");
  assert.equal(hydratedIntent.gen, 7, "hydrated intent gen must be 7");
  const nextGenAfterHydrate = pendingOverrideIntentStore.nextGen;
  assert.ok(nextGenAfterHydrate >= 8);
  mgr.isLoadComplete = false;
  const r = mgr.markChannelRead(channelId, undefined, 999);
  assert.equal(r.status, "queued");
  const freshIntent = pendingOverrideIntentStore.get(channelId);
  assert.ok(freshIntent, "fresh intent must be in store");
  assert.ok(freshIntent.gen > 7);
  assert.equal(freshIntent.readTarget, 999);
  mgr.effectiveState.set(channelId, 100); // frontier
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const frontier = mgr.effectiveState.get(channelId) ?? 0;
  assert.ok(frontier >= 999);
  assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  mgr.destroy();
});

// ── Test 54: scalar-number priorForcedEntry — hydrates as valid ForcedUnreadEntry ──
test("hydrate_scalarNumberPriorForcedEntry_acceptedAsValidLegacyEntry", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "54".repeat(32);
  const channelId1 = `scalar-num-ch-${"z".repeat(50)}`;
  const channelId2 = `sibling-scalar-${"A".repeat(49)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: {
        [channelId2]: { s: 2, c: 0, b: 30, f: 30 },
      },
      pi: {
        [channelId1]: { gen: 1, op: "unread", priorForcedEntry: 1700000000 },
        [channelId2]: { gen: 2, op: "unread" },
      },
      ng: 3,
    }),
  );
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  const intent1 = pendingOverrideIntentStore.get(channelId1);
  assert.ok(intent1, "ch1 intent (scalar-number prior) must hydrate");
  assert.equal(intent1.gen, 1, "ch1 intent gen must be 1");
  assert.equal(intent1.priorForcedEntry, 1700000000);
  const intent2 = pendingOverrideIntentStore.get(channelId2);
  assert.ok(intent2, "ch2 sibling intent must hydrate");
  assert.equal(intent2.gen, 2, "ch2 gen must be 2");
  const cb = wireDrainCallback(mgr, pubkey);
  mgr.effectiveState.set(channelId1, 50);
  mgr.effectiveState.set(channelId2, 30);
  await mgr.fetchAndMerge(mgr.loadGeneration);
  assert.equal(mgr.isLoadComplete, true);
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const reg2 = mgr.overrideRegisters.get(channelId2);
  assert.ok(reg2, "ch2 register must be present");
  const reg1 = mgr.overrideRegisters.get(channelId1);
  assert.ok(reg1, "ch1 register must be committed");
  assert.equal(reg1.s, 1, "ch1 S must be 1 after unread drain");
  assert.equal(pendingOverrideIntentStore.get(channelId1), undefined);
  assert.equal(pendingOverrideIntentStore.get(channelId2), undefined);
  cb.teardown();
  mgr.destroy();
});

// ── Test 55: backoff escalation — drain-abort retry delays escalate (1s→2s…) ──
test("scheduleAbortRetry_persistentFailure_escalatesBackoffAndRecoversBoundedly", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "55".repeat(32);
  const channelId = `backoff-escalation-ch-${"e".repeat(45)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;
  const origSetTimeout = globalThis.window.setTimeout;
  const origClearTimeout = globalThis.window.clearTimeout;
  const scheduledTimers = new Map(); // id → { fn, delay }
  let nextFakeId = 55000;
  globalThis.window.setTimeout = (fn, ms) => {
    if (ms > 0) {
      const id = nextFakeId++;
      scheduledTimers.set(id, { fn, delay: ms });
      return id;
    }
    return origSetTimeout(fn, ms);
  };
  globalThis.window.clearTimeout = (id) => {
    if (scheduledTimers.has(id)) {
      scheduledTimers.delete(id);
    } else {
      origClearTimeout(id);
    }
  };
  let blockV2Writes = false;
  const origSetItem = ls.setItem.bind(ls);
  ls.setItem = (key, value) => {
    if (blockV2Writes && key.startsWith("buzz.nip-rs.override-state.v2:")) {
      throw new Error("QuotaExceededError");
    }
    origSetItem(key, value);
  };
  const fakeRelay = makeFakeRelay();
  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    mgr.hydrateFromLocalStorage(); // clear any leftover state from previous tests
    mgr.effectiveState.set(channelId, 25);
    mgr.isLoadComplete = true;
    mgr.isLoadComplete = false; // queue via pending intents store
    mgr.markChannelUnread(channelId);
    mgr.isLoadComplete = true;
    blockV2Writes = true;
    await mgr.drainPendingIntents(mgr.loadGeneration);
    blockV2Writes = false; // still blocked during drain; unblock after
    assert.ok(mgr.drainRetryTimer !== null);
    const timerEntry1 = scheduledTimers.get(mgr.drainRetryTimer);
    assert.ok(timerEntry1, "abort retry timer ID must be in scheduledTimers");
    const delay1 = timerEntry1.delay;
    assert.equal(delay1, 1_000);
    assert.equal(mgr.drainRetryAttempt, 1);
    const retryTimerId1 = mgr.drainRetryTimer;
    const fn1 = timerEntry1.fn;
    blockV2Writes = true;
    scheduledTimers.delete(retryTimerId1);
    await fn1(); // drainRetryTimer fn fires; starts a drain synchronously
    blockV2Writes = false;
    assert.ok(mgr.drainRetryTimer !== null);
    const timerEntry2 = scheduledTimers.get(mgr.drainRetryTimer);
    assert.ok(timerEntry2);
    const delay2 = timerEntry2.delay;
    assert.equal(delay2, 2_000);
    assert.equal(mgr.drainRetryAttempt, 2);
    const fn3 = timerEntry2.fn;
    scheduledTimers.delete(mgr.drainRetryTimer);
    await fn3();
    await new Promise((r) => origSetTimeout(r, 0));
    const blobOk = JSON.parse(ls.getItem(v2Key) ?? "null") ?? {};
    assert.ok(blobOk.r?.[channelId]);
    assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
    assert.equal(mgr.drainRetryAttempt, 0);
    const frontier = mgr.effectiveState.get(channelId) ?? 0;
    assert.ok(frontier >= 25);
    assert.equal(mgr.drainRetryTimer, null);
    mgr.destroy();
  } finally {
    globalThis.window.setTimeout = origSetTimeout;
    globalThis.window.clearTimeout = origClearTimeout;
  }
});

// ── Test 56: stale-receipt collision at MAX_SAFE_INTEGER ceiling ───────────────
test("hydrate_maxSafeIntegerCeiling_staleReceiptCollision_replacementNotDiscarded", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "56".repeat(32);
  const channelId1 = `ceiling-ch1-${"f".repeat(52)}`;
  const channelId2 = `ceiling-ch2-${"g".repeat(52)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;
  const MAX = Number.MAX_SAFE_INTEGER;
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: {
        [channelId1]: { s: 3, c: 0, b: 100, f: 50 },
        [channelId2]: { s: 1, c: 0, b: 100, f: 30 },
      },
      receipts: {
        [channelId1]: { intentGen: 1, op: "read" },
      },
      pi: {
        [channelId1]: { gen: 1, op: "read", readTarget: 50 },
        [channelId2]: { gen: MAX, op: "unread" },
      },
      ng: MAX,
    }),
  );
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  const intent1 = pendingOverrideIntentStore.get(channelId1);
  assert.ok(intent1, "ch1 intent must hydrate after rebase");
  const intent2 = pendingOverrideIntentStore.get(channelId2);
  assert.ok(intent2, "ch2 intent must hydrate after rebase");
  assert.ok(Number.isSafeInteger(intent1.gen));
  assert.ok(Number.isSafeInteger(intent2.gen));
  const ng = pendingOverrideIntentStore.nextGen;
  assert.ok(Number.isSafeInteger(ng), "nextGen must be safe after rebase");
  assert.ok(ng > intent1.gen && ng > intent2.gen);
  mgr.isLoadComplete = false;
  const r = mgr.markChannelRead(channelId1, undefined, 999);
  assert.equal(r.status, "queued", "replacement read must be queued");
  const fresh1 = pendingOverrideIntentStore.get(channelId1);
  assert.ok(fresh1, "fresh intent must be in store");
  assert.ok(Number.isSafeInteger(fresh1.gen));
  mgr.effectiveState.set(channelId1, 50);
  mgr.effectiveState.set(channelId2, 30);
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const frontier1 = mgr.effectiveState.get(channelId1) ?? 0;
  assert.ok(frontier1 >= 999);
  assert.equal(pendingOverrideIntentStore.get(channelId1), undefined);
  mgr.destroy();
});

// ── Test 57: ng=MAX_SAFE_INTEGER multi-enqueue — no unsafe gen minted ────────
test("enqueue_ngMaxSafeInteger_twoEnqueues_noUnsafeGenMinted_bothSurviveHydration", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "57".repeat(32);
  const ch1 = `max-enq-ch1-${"h".repeat(52)}`;
  const ch2 = `max-enq-ch2-${"i".repeat(52)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;
  const MAX = Number.MAX_SAFE_INTEGER;
  ls.setItem(v2Key, JSON.stringify({ ng: MAX }));
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  const ngAfterHydrate = pendingOverrideIntentStore.nextGen;
  assert.ok(Number.isSafeInteger(ngAfterHydrate));
  assert.ok(ngAfterHydrate < MAX);
  mgr.isLoadComplete = false;
  let r1;
  assert.doesNotThrow(() => {
    r1 = mgr.markChannelUnread(ch1);
  }, "first enqueue must not throw");
  assert.equal(r1.status, "queued", "first enqueue must be queued");
  const intent1 = pendingOverrideIntentStore.get(ch1);
  assert.ok(intent1, "first intent must be in store");
  assert.ok(Number.isSafeInteger(intent1.gen));
  let r2;
  assert.doesNotThrow(() => {
    r2 = mgr.markChannelUnread(ch2);
  }, "second enqueue must not throw");
  assert.equal(r2.status, "queued", "second enqueue must be queued");
  const intent2 = pendingOverrideIntentStore.get(ch2);
  assert.ok(intent2, "second intent must be in store");
  assert.ok(Number.isSafeInteger(intent2.gen));
  assert.notEqual(intent1.gen, intent2.gen);
  const blobRaw = ls.getItem(v2Key);
  assert.ok(blobRaw, "v2 blob must be present");
  const blob = JSON.parse(blobRaw);
  assert.ok(blob.pi?.[ch1], "ch1 pi must be in blob");
  assert.ok(blob.pi?.[ch2], "ch2 pi must be in blob");
  const mgr2 = new ReadStateManager(pubkey, fakeRelay);
  mgr2.hydrateFromLocalStorage();
  assert.ok(pendingOverrideIntentStore.get(ch1));
  assert.ok(pendingOverrideIntentStore.get(ch2));
  mgr2.destroy();
  mgr.destroy();
});

// ── Test 58: thrown-handler — callback throws, tx aborts, bounded retry, latch cleared ──
test("drain_onDrainOutcomeThrows_transactionAborts_gen1Restored_boundedRetryScheduled", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "58".repeat(32);
  const channelId = `thrown-cb-ch-${"j".repeat(51)}`;
  const origSetTimeout = globalThis.window.setTimeout;
  const origClearTimeout = globalThis.window.clearTimeout;
  const scheduledTimers = new Map();
  let nextFakeId = 58000;
  globalThis.window.setTimeout = (fn, ms) => {
    if (ms > 0) {
      const id = nextFakeId++;
      scheduledTimers.set(id, { fn, delay: ms });
      return id;
    }
    return origSetTimeout(fn, ms);
  };
  globalThis.window.clearTimeout = (id) => {
    if (scheduledTimers.has(id)) scheduledTimers.delete(id);
    else origClearTimeout(id);
  };
  const fakeRelay = makeFakeRelay();
  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    mgr.hydrateFromLocalStorage(); // clear any leftover state from previous tests
    mgr.effectiveState.set(channelId, 50);
    mgr.isLoadComplete = false; // Queue via pending intents store
    mgr.markChannelUnread(channelId);
    const gen1Intent = pendingOverrideIntentStore.get(channelId);
    assert.ok(gen1Intent, "gen1 must be in store after markChannelUnread");
    const gen1 = gen1Intent.gen;
    let callbackFired = false;
    mgr.onDrainOutcome = (_outcome) => {
      callbackFired = true;
      throw new Error("simulated callback failure");
    };
    mgr.isLoadComplete = true;
    await mgr.drainPendingIntents(mgr.loadGeneration);
    assert.ok(callbackFired, "onDrainOutcome must have been invoked");
    const intentAfterThrow = pendingOverrideIntentStore.get(channelId);
    assert.ok(intentAfterThrow);
    assert.equal(intentAfterThrow.gen, gen1, "restored intent must be gen1");
    mgr.isLoadComplete = false;
    const rAfterAbort = mgr.markChannelRead(channelId, undefined, 999);
    assert.notEqual(rAfterAbort.status, undefined);
    const intentAfterEnqueue = pendingOverrideIntentStore.get(channelId);
    assert.ok(intentAfterEnqueue, "intent must exist after enqueue");
    assert.ok(intentAfterEnqueue.gen > 0);
    assert.ok(mgr.drainRetryTimer !== null);
    const retryEntry = scheduledTimers.get(mgr.drainRetryTimer);
    assert.ok(retryEntry, "abort retry timer must be in scheduledTimers");
    assert.equal(retryEntry.delay, 1_000);
    mgr.destroy();
  } finally {
    globalThis.window.setTimeout = origSetTimeout;
    globalThis.window.clearTimeout = origClearTimeout;
  }
});

// ── Test 59: identity swap — A deferred mid-tx, B hydrates, B sees no A state ──
test("identitySwap_aDeferredMidTransaction_bHydrates_bSeesNoAState", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkeyA = "59aa".repeat(16);
  const pubkeyB = "59bb".repeat(16);
  const channelId = `id-swap-ch-${"k".repeat(53)}`;
  const v2KeyB = `buzz.nip-rs.override-state.v2:${pubkeyB}`;
  const ls = globalThis.window.localStorage;
  const fakeRelay = makeFakeRelay();
  const mgrA = new ReadStateManager(pubkeyA, fakeRelay);
  mgrA.effectiveState.set(channelId, 10);
  mgrA.isLoadComplete = false;
  mgrA.markChannelUnread(channelId);
  pendingOverrideIntentStore.beginTransaction(channelId);
  mgrA.markChannelRead(channelId, undefined, 999);
  ls.setItem(v2KeyB, JSON.stringify({ ng: 2 }));
  const mgrB = new ReadStateManager(pubkeyB, fakeRelay);
  mgrB.hydrateFromLocalStorage();
  mgrB.isLoadComplete = false;
  const rB = mgrB.markChannelUnread(channelId);
  assert.equal(rB.status, "queued", "B enqueue must be queued");
  const intentB = pendingOverrideIntentStore.get(channelId);
  assert.ok(intentB, "B must have its own intent after enqueue");
  assert.ok(intentB.gen > 0);
  assert.ok(Number.isSafeInteger(intentB.gen));
  assert.equal(intentB.op, "unread");
  assert.equal(intentB.readTarget, undefined);
  mgrA.destroy();
  mgrB.destroy();
});

// ── Test 60: MAX-2 boundary — two enqueues from valid two-below-ceiling hydration ──
test("hydrate_maxMinusTwoState_twoEnqueues_safeGensNeitherThrows_ngPersistSafe", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "60".repeat(32);
  const channelId = `max-minus-two-ch-${"\u006d".repeat(47)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;
  const MAX = Number.MAX_SAFE_INTEGER;
  const fakeRelay = makeFakeRelay();
  ls.setItem(
    v2Key,
    JSON.stringify({
      pi: { [channelId]: { gen: MAX - 2, op: "unread" } },
      ng: MAX - 2,
    }),
  );
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  const ngAfterHydrate = pendingOverrideIntentStore.nextGen;
  assert.ok(Number.isSafeInteger(ngAfterHydrate));
  assert.ok(ngAfterHydrate < MAX);
  assert.equal(ngAfterHydrate, 2, "nextGen must be 2 after rebase of 1 intent");
  const ch2 = `max-minus-two-ch2-${"n".repeat(46)}`;
  mgr.isLoadComplete = false;
  let r1;
  assert.doesNotThrow(() => {
    r1 = mgr.markChannelUnread(ch2);
  }, "first enqueue must not throw");
  assert.equal(r1.status, "queued", "first enqueue must be queued");
  const intent1 = pendingOverrideIntentStore.get(ch2);
  assert.ok(intent1, "first intent must be in store");
  assert.ok(Number.isSafeInteger(intent1.gen));
  assert.ok(intent1.gen > 0, "first intent gen must be positive");
  const blob1Raw = ls.getItem(v2Key);
  assert.ok(blob1Raw, "v2 blob must be present after first enqueue");
  const blob1 = JSON.parse(blob1Raw);
  const ng1 = blob1.ng ?? 1; // ng omitted when === 1 (the `ng > 1` condition)
  assert.ok(Number.isSafeInteger(ng1));
  assert.ok(ng1 < MAX);
  const ch3 = `max-minus-two-ch3-${"o".repeat(46)}`;
  let r2;
  assert.doesNotThrow(() => {
    r2 = mgr.markChannelRead(ch3, undefined, 500);
  }, "second enqueue must not throw");
  assert.equal(r2.status, "queued", "second enqueue must be queued");
  const intent2 = pendingOverrideIntentStore.get(ch3);
  assert.ok(intent2, "second intent must be in store");
  assert.ok(Number.isSafeInteger(intent2.gen));
  assert.notEqual(intent1.gen, intent2.gen);
  const blob2Raw = ls.getItem(v2Key);
  assert.ok(blob2Raw, "v2 blob must be present after second enqueue");
  const blob2 = JSON.parse(blob2Raw);
  const ng2 = blob2.ng ?? 1;
  assert.ok(Number.isSafeInteger(ng2));
  assert.ok(ng2 < MAX);
  const mgr2 = new ReadStateManager(pubkey, fakeRelay);
  mgr2.hydrateFromLocalStorage();
  assert.ok(pendingOverrideIntentStore.get(ch2));
  assert.ok(pendingOverrideIntentStore.get(ch3));
  mgr2.destroy();
  mgr.destroy();
});

// ── Test 61: promotion allocation failure — no stranded lock, retry scheduled ──
test("drain_promoteDeferredAllocExhausted_noStrandedLock_retryScheduled_drainScheduledCleared", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "61".repeat(32);
  const channelId = `promo-alloc-ch-${"p".repeat(50)}`;
  const MAX = Number.MAX_SAFE_INTEGER;
  const timers = makeFakeTimers();
  const fakeRelay = makeFakeRelay();
  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    mgr.effectiveState.set(channelId, 50);
    const gen1Intent = { gen: 1, op: /** @type {"unread"} */ ("unread") };
    pendingOverrideIntentStore.restoreFromStorage(
      new Map([[channelId, gen1Intent]]),
      MAX,
    );
    mgr.onDrainOutcome = (outcome) => {
      if (
        outcome.kind === "applied-unread" &&
        outcome.channelId === channelId
      ) {
        const prev = mgr.isLoadComplete;
        mgr.isLoadComplete = false; // route through deferred latch path
        mgr.markChannelRead(channelId, undefined, 999);
        mgr.isLoadComplete = prev;
      }
    };
    mgr.isLoadComplete = true;
    await mgr.drainPendingIntents(mgr.loadGeneration);

    // ── drainScheduled must be false (manager's try-finally always clears it) ──
    assert.equal(mgr.drainScheduled, false);

    // ── drainRetryTimer must be set (scheduleAbortRetry called from catch) ──
    assert.ok(mgr.drainRetryTimer !== null);
    assert.ok(timers.scheduled.has(mgr.drainRetryTimer));

    // ── Channel is unlocked: direct probe of lockedChannels ──
    assert.equal(
      pendingOverrideIntentStore.lockedChannels.has(channelId),
      false,
    );

    // ── gen1 must be the live intent (abortTransaction restored it) ──
    const intentAfterFail = pendingOverrideIntentStore.get(channelId);
    assert.ok(intentAfterFail);
    assert.equal(intentAfterFail.gen, gen1Intent.gen);

    // ── Original deferred gen2 payload must be retained in deferredEnqueues ──
    const deferredPayload =
      pendingOverrideIntentStore.deferredEnqueues.get(channelId);
    assert.ok(deferredPayload);
    assert.equal(deferredPayload.op, "read");
    assert.equal(deferredPayload.readTarget, 999);

    // ── No promoted payload must exist ──
    assert.equal(
      pendingOverrideIntentStore.promotedDeferredPayloads.get(channelId),
      undefined,
    );

    // ── Recovery: reset _nextGen directly to a safe value WITHOUT hydrating ──
    pendingOverrideIntentStore._nextGen = 2;
    mgr.onDrainOutcome = null;
    const retryTimerId = mgr.drainRetryTimer;
    const retryFn = timers.scheduled.get(retryTimerId)?.fn;
    assert.ok(retryFn, "retry timer fn must be in timers.scheduled");
    timers.scheduled.clear();
    await retryFn();
    await new Promise((r) => timers.origSetTimeout(r, 0));

    // ── After retry: gen1 cleaned up, gen2 (from retained payload) committed ──
    const frontierAfterRetry = mgr.effectiveState.get(channelId) ?? 0;
    assert.ok(frontierAfterRetry >= 999);
    assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
    assert.equal(mgr.drainScheduled, false);
    mgr.destroy();
  } finally {
    timers.restore();
  }
});

// ── Test 62: normal blob — no rebase fires on healthy state ──────────────────
test("hydrate_normalBlob_noRebase_verbatimNextGen", () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "62".repeat(32);
  const channelId = `normal-blob-ch-${"n".repeat(49)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;
  const fakeRelay = makeFakeRelay();
  ls.setItem(
    v2Key,
    JSON.stringify({
      pi: { [channelId]: { gen: 1000, op: "unread" } },
      ng: 1001,
    }),
  );
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  const ngAfterHydrate = pendingOverrideIntentStore.nextGen;
  assert.equal(ngAfterHydrate, 1001);
  const intent = pendingOverrideIntentStore.get(channelId);
  assert.ok(intent, "intent must be present after hydration");
  assert.equal(intent.gen, 1000);
  mgr.destroy();
});

// ── Test 63: pre-rebase receipt sweep — stale receipt does not manufacture alreadyApplied ──
test("hydrate_rebase_staleReceiptSweptBeforeCompaction_drainReplaysAction", async () => {
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "63".repeat(32);
  const channelId = `stale-receipt-ch-${"s".repeat(47)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;
  const MAX = Number.MAX_SAFE_INTEGER;
  const HEADROOM = 2 ** 32;
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: {
        [channelId]: { s: 4, c: 2, b: 0, f: 0 },
      },
      receipts: {
        [channelId]: { intentGen: 1, op: "unread" },
      },
      pi: {
        [channelId]: { gen: MAX - HEADROOM, op: "unread" },
      },
      ng: MAX - HEADROOM,
    }),
  );
  const fakeRelay = makeFakeRelay();
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  const intentAfterHydrate = pendingOverrideIntentStore.get(channelId);
  assert.ok(intentAfterHydrate, "intent must survive hydration");
  assert.ok(Number.isSafeInteger(intentAfterHydrate.gen));
  assert.equal(intentAfterHydrate.op, "unread", "intent op must be unread");
  mgr.effectiveState.set(channelId, 0);
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  const reg = mgr.overrideRegisters.get(channelId);
  assert.ok(reg, "register must exist after drain");
  assert.equal(reg.s, 5);
  assert.equal(pendingOverrideIntentStore.get(channelId), undefined);
  const blobAfterDrain = JSON.parse(ls.getItem(v2Key) ?? "{}");
  const receiptInBlob = blobAfterDrain?.receipts?.[channelId] ?? null;
  assert.equal(receiptInBlob, null);
  mgr.destroy();
});
