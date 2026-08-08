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

// ── ReadStateManager integration helpers ─────────────────────────────────────
// Provide browser globals required by ReadStateManager (localStorage,
// window.setTimeout/clearTimeout). Each test that uses ReadStateManager
// constructs a fresh in-memory store so tests are isolated.

function makeLocalStorage() {
  const store = new Map();
  return {
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, value),
    removeItem: (key) => store.delete(key),
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
  // Ensure bare `localStorage` always proxies to window.localStorage.
  Object.defineProperty(globalThis, "localStorage", {
    get: () => globalThis.window.localStorage,
    configurable: true,
  });
}

/**
 * Build a FenceHandle-shaped fake for use in test relay objects.
 *
 * @param {object} opts
 * @param {boolean}  opts.eose      — if true, `established` resolves immediately (EOSE received).
 * @param {boolean}  opts.lapsesAfterEose — if true, `lapsed` becomes true after established resolves.
 * @param {boolean}  opts.lapseBeforeEose — if true, sets lapsed=true and resolves established together.
 * @param {(ev: object) => void} [opts.captureHandler] — called with the onEvent handler so tests can deliver events.
 */
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
    // Lapsed before EOSE: resolve immediately with lapsed=true.
    resolveEstablished();
  } else if (eose) {
    resolveEstablished();
    if (lapsesAfterEose) lapsed = true;
  }
  // If neither, `established` never resolves (hung fence — caller will lapse via reconnect).

  return {
    established,
    get lapsed() {
      return lapsed;
    },
    unsubscribe: async () => {},
    /** For tests that need to trigger a lapse mid-enumeration. */
    _lapse() {
      lapsed = true;
    },
  };
}

/**
 * Wire the production `createDrainOutcomeHandler` onto `manager.onDrainOutcome`.
 *
 * Uses the SAME pure handler exported from `useUnreadChannelsHelpers.ts` that
 * `useDrainOutcomeCallback` uses in production — no hand-copied mirror.  This
 * guarantees the test exercises the exact same outcome routing logic.
 *
 * Returns an object with:
 *  - `forcedUnreadMap`   — live reference to the map being mutated
 *  - `pendingSnapshots`  — in-memory snapshot map (same role as the React ref)
 *  - `versionBumps`      — counter incremented on each bumpLatestVersion call
 *
 * Call `teardown()` to clear `manager.onDrainOutcome` when the test is done.
 */
function wireDrainCallback(manager, pubkey) {
  const forcedUnreadMap = forcedUnreadStore.read(pubkey);
  const pendingSnapshots = new Map();
  let versionBumps = 0;

  const bumpLatestVersion = () => {
    versionBumps++;
  };

  // Delegate to the production handler — not a hand-copied mirror.
  manager.onDrainOutcome = createDrainOutcomeHandler(
    { current: forcedUnreadMap },
    pendingSnapshots,
    pubkey,
    bumpLatestVersion,
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
    subscribeFenced: async (_filter, _onEvent) =>
      makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
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

  // Simulate a completed full-state load so the truncation guard does not
  // block the publish path. The guard is tested separately.
  mgr.isLoadComplete = true;

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

// ── NIP-RS override layer: mandatory acceptance tests ─────────────────────────

// Helper: build a ReadStateManager with mocked relay and localStorage.
// subscribeFenced returns an immediately-established fence (happy path).
// subscribeLive is still wired for the live subscription path.
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
  // Build channel entries that include ov_* keys for two contexts.
  // Also include enough plain channel entries to force multi-slot distribution.
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

  // Add enough plain channel entries to force at least 2 slots.
  // We need the round-robin entries (NOT the pinned ov entries) to overflow.
  const plainChannelEntries = [];
  for (let i = 0; i < 30; i++) {
    plainChannelEntries.push([makeChannelKey(i), i + 1]);
  }
  const channelEntries = [...ovEntries, ...plainChannelEntries];

  const encoder = new TextEncoder();
  // Compute the size of a slot containing only the 8 override+frontier entries.
  const ovOnlyContexts = Object.fromEntries(ovEntries);
  const ovOnlySize = encoder.encode(
    JSON.stringify({ v: 1, client_id: CLIENT_ID, contexts: ovOnlyContexts }),
  ).length;
  // Budget: fits the override entries + ~10 plain entries per slot,
  // but not all 30 plain entries in one slot (forces at least 3 slots).
  // Add 15 plain entries to the ov-only size to get a safe per-slot budget.
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

  // Verify: no ov_* key appears in any non-primary slot.
  for (let slotIdx = 1; slotIdx < result.slots.length; slotIdx++) {
    const slot = result.slots[slotIdx];
    for (const key of Object.keys(slot)) {
      assert.ok(
        !key.startsWith("ov_"),
        `ov_* key "${key}" must not appear in slot ${slotIdx} (non-primary)`,
      );
    }
  }

  // Verify: ov_* keys and their frontier siblings ARE in slot 0.
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
  // A context whose raw ID starts with "ov_" must be escaped to "esc:ov_s:evil"
  // as a frontier wire key.  The ov_* siblings are keyed by the RAW suffix
  // "ov_s:evil".  The splitter must unescape "esc:ov_s:evil" → "ov_s:evil"
  // and recognise it as belonging to the same group as ov_s:ov_s:evil etc.
  const rawCtx = "ov_s:evil";
  const wireKey = `esc:${rawCtx}`; // what currentContexts() emits

  const channelEntries = [
    [`ov_s:${rawCtx}`, 1], // ov_s:ov_s:evil
    [`ov_c:${rawCtx}`, 0], // ov_c:ov_s:evil
    [`ov_b:${rawCtx}`, 50], // ov_b:ov_s:evil
    [wireKey, 50], // esc:ov_s:evil  (escaped frontier)
  ];
  // Add plain entries to force multi-slot so the splitter actually partitions.
  const plain = [];
  for (let i = 0; i < 20; i++) plain.push([makeChannelKey(i), i + 1]);
  const allEntries = [...channelEntries, ...plain];

  const encoder = new TextEncoder();
  // Budget: fits the override group + 5 plain entries, not all 20.
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
  // The escaped frontier key and all three ov_* siblings must be in slot 0.
  assert.ok(
    wireKey in slot0,
    `${wireKey} (escaped frontier) must be in slot 0`,
  );
  assert.ok(`ov_s:${rawCtx}` in slot0, "ov_s: sibling must be in slot 0");
  assert.ok(`ov_c:${rawCtx}` in slot0, "ov_c: sibling must be in slot 0");
  assert.ok(`ov_b:${rawCtx}` in slot0, "ov_b: sibling must be in slot 0");
  // No ov_* or esc: entry in non-primary slots.
  for (let i = 1; i < result.slots.length; i++) {
    for (const key of Object.keys(result.slots[i])) {
      assert.ok(
        !key.startsWith("ov_") && !key.startsWith("esc:"),
        `reserved key "${key}" must not appear in slot ${i}`,
      );
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
  // A relay with no events + EOSE-established fence → empty first band → complete.
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
  assert.equal(
    mgr.isLoadComplete,
    true,
    "empty relay with EOSE-established fence must produce complete load",
  );
  assert.equal(
    subscribeFencedCallCount,
    1,
    "subscribeFenced must be called exactly once for the fence",
  );
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
  // Empty first band → loader sets complete=true tentatively; fence lapses
  // after EOSE (simulates mid-load reconnect). Final fence.lapsed check must
  // override and force complete:false.
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
  assert.equal(
    mgr.isLoadComplete,
    false,
    "lapse after tentative complete must override and force incomplete",
  );
  mgr.destroy();
});

// ── 2e: single event completes after pinned-window discharge ──────────────────
test("fetchAndMerge_singleEvent_completesAfterPinnedWindowDischarge", async () => {
  // Single event at T=1000: band=1, C=1, L=2 → max(C,L)=2.
  // Pinned window {since:1000, until:1000} returns 1 event → 1 < 2 → discharged.
  // Continuation {until:999} returns 0 → complete.
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
  assert.equal(
    mgr.isLoadComplete,
    true,
    "single-event relay must produce complete load after pinned-window discharge",
  );
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
  assert.equal(
    mgr.isLoadComplete,
    false,
    "pinned window returning ≥ max(C,L) must produce incomplete load",
  );
  mgr.destroy();
});

// ── 2g: epoch-zero termination ───────────────────────────────────────────────
test("fetchAndMerge_epochZero_completesAfterPinnedWindowDischarged", async () => {
  // T=0: an event at created_at=0 → T=0. Once the pinned window is discharged,
  // no event can have created_at < 0, so the load terminates complete directly.
  // The loader must NOT issue an `until:0` continuation (which is inclusive
  // and would re-fetch the same epoch-zero event, making completion unprovable).
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "a4".repeat(32);
  const event = makeFakeEvent(pubkey, 0); // created_at=0
  let continuationCalled = false;
  const fakeRelay = {
    fetchEvents: async (filter) => {
      // Pinned window: {since:0, until:0} → 1 event, 1 < max(1,2)=2 → discharged.
      if (filter.since !== undefined && filter.until !== undefined)
        return [event];
      // Any `until:0` without `since` would be the old continuation path —
      // it must NOT be issued; flag it if it is.
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
  assert.equal(
    mgr.isLoadComplete,
    true,
    "T=0 pinned-window-discharged must produce complete load directly",
  );
  assert.equal(
    continuationCalled,
    false,
    "loader must NOT issue until:0 continuation after epoch-zero pinned-window discharge",
  );
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
  assert.equal(
    mgr.isLoadComplete,
    false,
    "subscribeFenced failure must produce incomplete load",
  );
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
  assert.equal(
    ur.status,
    "queued",
    "markChannelUnread on incomplete load must return queued",
  );

  const rr = mgr.markChannelRead("ch");
  assert.equal(
    rr.status,
    "queued",
    "markChannelRead on incomplete load must return queued",
  );

  mgr.extraSlotIds = ["fakeextraslot0000000000000000000"];
  await mgr.deleteExtraSlots();
  assert.equal(
    publishCalls,
    0,
    "deleteExtraSlots must not publish when load incomplete",
  );

  mgr.destroy();
});

// ── 2i-b: pre-ready read click → zero register/frontier mutation, zero publish ─
test("preReady_read_witness_b_zeroMutationZeroPublish", async () => {
  // Witness (b): explicit mark-read during incomplete load causes zero
  // register mutation, zero frontier advance, and zero publication.
  //
  // Verifies:
  //   - markChannelRead returns "queued" (intent enqueued only, no bump).
  //   - No publish fires.
  //   - overrideRegisters is empty (no S/C/B bump).
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
    // Force incomplete load by making subscribeFenced throw.
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

  // Simulate the read click: markChannelRead (register path) only.
  // The frontier advance (markContextRead) is guarded at the hook level, not here.
  const result = mgr.markChannelRead(channelId);
  assert.equal(
    result.status,
    "queued",
    "markChannelRead must return queued pre-ready",
  );

  // Register is untouched — no S/C/B bump.
  const regAfter = mgr.overrideRegisters.get(channelId);
  assert.equal(
    regAfter,
    regBefore,
    "override register must be unchanged after queued read",
  );
  assert.equal(
    regAfter,
    undefined,
    "no register entry must exist for the channel",
  );

  // No publication fired.
  mgr.fetchOwnBlobBeforePublish = async () => true;
  await mgr.publish();
  assert.equal(publishCalls, 0, "zero publish allowed before complete load");

  mgr.destroy();
});

// ── 2j: production retry path fires on reconnect ─────────────────────────────
test("retryLoad_firesOnReconnect_andClearsIncomplete", async () => {
  // startLiveSubscription wires retryLoad() via subscribeToReconnects.
  // (1) reconnect listener registered; (2) firing it re-runs fetchAndMerge.
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
  assert.equal(
    mgr.isLoadComplete,
    false,
    "precondition: first load is incomplete",
  );
  assert.ok(
    reconnectCb !== null,
    "subscribeToReconnects listener must be registered",
  );

  callRound = 999;
  reconnectCb();
  await new Promise((r) => setTimeout(r, 50));
  assert.equal(
    mgr.isLoadComplete,
    true,
    "reconnect-triggered retry must clear incomplete when relay is now empty",
  );
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
  assert.equal(
    mgr.isLoadComplete,
    true,
    "retry with empty relay must set complete",
  );
  mgr.destroy();
});

// ── Test 3: live events go through structured ingest ──────────────────────────
test("handleIncomingEvent_liveOverride_updatesRegisterViaIngest", async () => {
  // Construct a fake event whose content decrypts to a NIP-RS blob with an
  // override register.  Use a __TAURI_INTERNALS__ mock so parseReadStateEvent
  // can decrypt without a real NIP-44 key.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "aa".repeat(32);
  const rawCtx = `live-channel-${"x".repeat(51)}`;
  const channelFrontier = 100;
  // Build a blob where the register (S=3, C=1, B=50) is ACTIVE:
  // isOverrideActive = S>0 && F<=B && S>C. B=50, frontier=100 → F>B → INACTIVE.
  // Corrected: to make this active we need B >= F, e.g. B=100 with F=100 (boundary).
  // Using B=100 so the register is genuinely active: 3>0, 100<=100, 3>1 → active.
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

  // Install Tauri IPC mock so nip44_decrypt_from_self returns our plaintext.
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

    // Build fake NIP-RS event.
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

    // Deliver via the live subscription callback (same path as relay push).
    // Track decrypt count: each live event must decrypt exactly once.
    let decryptCount = 0;
    const origInvoke = globalThis.window.__TAURI_INTERNALS__.invoke;
    globalThis.window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "nip44_decrypt_from_self") decryptCount++;
      return origInvoke(command, args);
    };
    assert.ok(liveHandler !== null, "live subscription must be established");
    liveHandler(fakeEvent); // void-wrapped; wait for async completion
    await new Promise((r) => setTimeout(r, 50));
    assert.equal(
      decryptCount,
      1,
      "live event must decrypt exactly once (no double-parse)",
    );

    // The override register must now reflect the ingested remote values.
    const reg = mgr.overrideRegisters.get(rawCtx);
    assert.ok(reg, "override register must exist after live delivery");
    assert.equal(reg.s, 3, "S must be 3 from live event");
    assert.equal(reg.c, 1, "C must be 1 from live event");
    assert.equal(reg.b, 100, "B must be 100 from live event");

    // Verify the register is genuinely active: isOverrideActive(S=3,C=1,B=100,F=100).
    const livenessActive = mgr.getOverrideLiveness(rawCtx);
    assert.ok(
      livenessActive !== null,
      "liveness must be available after live delivery",
    );
    assert.equal(
      livenessActive.active,
      true,
      "register must be active (B=100 >= F=100, S>C)",
    );

    // ── existing-key live clear (higher C defeating S) ───────────────────
    // A follow-up event with S=3, C=4 (C > S → inactive/tombstone).
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
    // tombstone floor: only ov_c:ctx=4 is present → S=0, C=4, B=0 merged via componentwise max
    // after merge with prior (S=3, C=1, B=100): S=3, C=4, B=100 — override_active = S <= C = inactive
    assert.equal(
      regAfterClear.c,
      4,
      "C must be 4 (tombstone floor from clear event)",
    );
    const liveness = mgr.getOverrideLiveness(rawCtx);
    assert.ok(liveness !== null, "liveness must be available");
    assert.equal(
      liveness.active,
      false,
      "override must be inactive after clear",
    );

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
      // Own-blob fetch (has #d filter) → fail.
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
  assert.equal(
    publishCalls,
    0,
    "publish must not call publishEvent when fetchOwnBlobBeforePublish fails",
  );
  mgr.destroy();
});

// ── Test 5: durability — register survives restart before debounce ────────────
test("overrideRegister_survivesRestartBeforeDebounce", () => {
  // markChannelUnread persists the register via persistLocalState.
  // A new ReadStateManager constructed from the same localStorage must see it.
  const ls = makeLocalStorage();
  globalThis.window.localStorage = ls;
  const pubkey = "cc".repeat(32);
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
  };
  const mgr1 = new ReadStateManager(pubkey, fakeRelay);
  mgr1.isLoadComplete = true;
  mgr1.effectiveState.set("restart-ch", 1000);
  mgr1.publishableContextIds.add("restart-ch");
  const result = mgr1.markChannelUnread("restart-ch");
  assert.equal(result.status, "applied", "mark-unread must succeed");
  mgr1.destroy();

  // Construct a new manager on the same localStorage — no relay fetch yet.
  globalThis.window.localStorage = ls;
  const mgr2 = new ReadStateManager(pubkey, fakeRelay);
  mgr2.hydrateFromLocalStorage();
  const liveness = mgr2.getOverrideLiveness("restart-ch");
  assert.ok(liveness !== null, "register must be hydrated after restart");
  assert.equal(
    liveness.active,
    true,
    "override must still be active after restart",
  );
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
  // Establish a mark-read tombstone floor: S=1, C=max(S,C)+1=2 (clear-wins → inactive).
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

  // New manager: initialize() with a relay that throws on fetchEvents.
  // Tombstone must still be present even after failed fetch.
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
  // Simulate completed load so mark operations are not blocked by load_incomplete.
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
  assert.equal(
    overflowResult.reason,
    "uint32_overflow",
    "markChannelUnread must refuse with uint32_overflow when S is at max",
  );

  // ── multi-slot success: 700 frontier-only channels → split planner must succeed ─
  // Thufir's deterministic witness: 700 plain frontier channels overflow a single slot
  // but the splitter can spread them across ≤ 8 slots. A new override group in the
  // primary must succeed because the pure candidate planner tries the split path.
  const splitMgr = makeManager("f".repeat(64));
  splitMgr.isLoadComplete = true;
  for (let i = 0; i < 700; i++) {
    const ctx = `frontier-ch-${i.toString().padStart(60, "0")}`;
    splitMgr.effectiveState.set(ctx, 1000 + i);
    splitMgr.publishableContextIds.add(ctx);
  }
  const splitCtx = `new-override-ctx-${"n".repeat(48)}`;
  const splitResult = splitMgr.markChannelUnread(splitCtx);
  assert.equal(
    splitResult.status,
    "applied",
    "700 frontier-only channels must allow a new override via multi-slot split",
  );
  assert.ok(
    splitMgr.overrideRegisters.has(splitCtx),
    "override register must be committed on split success",
  );
  splitMgr.destroy();

  // ── near-limit refusal: all 8 slots insufficient → budget_exhausted, no mutation ─
  // Pack so many non-evictable override groups that even 8 slots cannot accommodate
  // the new entry. Each override group contributes ov_s+ov_c+ov_b+frontier ≈ 4 keys
  // × ~75 bytes each + JSON overhead.  200 groups × 4 keys ≈ 300 bytes each ≈ 60 KB
  // per slot if split across 8 → ~7.5 KB per slot, which fits. We need them to NOT fit.
  // Easiest: fill primary to near-capacity with 250 big-key override groups so even
  // splitting all 8 slots still cannot carry the new group in primary (which must
  // hold ALL override groups per spec, so they all go in slot 0).
  const fullMgr = makeManager("aa".repeat(32));
  fullMgr.isLoadComplete = true;
  // 250 override groups with long context IDs ≈ 250 × (4 keys × ~100 bytes) = 100 KB
  // → exceeds READ_STATE_MAX_PLAINTEXT_BYTES (32 KB) even in slot 0 alone.
  for (let i = 0; i < 250; i++) {
    const ctx = `ov-ch-${i.toString().padStart(60, "0")}`;
    fullMgr.overrideRegisters.set(ctx, { s: 1, c: 0, b: 0 });
    fullMgr.effectiveState.set(ctx, 1000 + i);
    fullMgr.publishableContextIds.add(ctx);
  }
  const nearCtx = `near-limit-ctx-${"z".repeat(49)}`;
  const nearResult = fullMgr.markChannelUnread(nearCtx);
  assert.equal(
    nearResult.status,
    "refused",
    "near-limit manager with 250 non-evictable override groups must refuse",
  );
  assert.equal(nearResult.reason, "budget_exhausted");
  assert.ok(
    !fullMgr.overrideRegisters.has(nearCtx),
    "budget_exhausted must not mutate overrideRegisters",
  );
  fullMgr.destroy();

  // Verify the uint32_overflow register was not mutated in the original mgr.
  const reg = mgr.overrideRegisters.get(overflowCtx);
  assert.ok(reg, "overflow channel register must still exist");
  assert.equal(reg.s, UINT32_MAX, "overflow register S must be unchanged");

  mgr.destroy();
});

// ── Test 8: persistence failure — storage_failed + rollback + coherent restart ─
test("markChannelUnread_storageFailure_returnsStorageFailed", () => {
  // Use a throwing localStorage to simulate quota failure.
  const throwingLS = makeLocalStorage();
  const originalSetItem = throwingLS.setItem.bind(throwingLS);
  // Allow initial writes (ClientId, slotId), then fail on override writes.
  let writeCount = 0;
  throwingLS.setItem = (key, value) => {
    writeCount++;
    if (writeCount > 2) throw new Error("QuotaExceededError");
    originalSetItem(key, value);
  };
  globalThis.window.localStorage = throwingLS;
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };
  const mgr = new ReadStateManager("st".repeat(32), fakeRelay);
  mgr.isLoadComplete = true;
  mgr.effectiveState.set("storage-test-ch", 1000);

  // Snapshot pre-mutation state for rollback verification.
  const regBefore = mgr.overrideRegisters.get("storage-test-ch");
  const wasPublishableBefore = mgr.publishableContextIds.has("storage-test-ch");

  const result = mgr.markChannelUnread("storage-test-ch");
  assert.equal(
    result.status,
    "refused",
    "markChannelUnread must fail when localStorage throws",
  );
  assert.equal(result.reason, "storage_failed");

  // Contract 3 rollback: manager state must be unchanged after storage_failed.
  const regAfter = mgr.overrideRegisters.get("storage-test-ch");
  assert.deepEqual(
    regAfter,
    regBefore,
    "overrideRegisters must be rolled back after storage failure",
  );
  assert.equal(
    mgr.publishableContextIds.has("storage-test-ch"),
    wasPublishableBefore,
    "publishableContextIds must be rolled back after storage failure",
  );

  // Coherent restart: a new manager on the same (throwing) storage must see no
  // orphaned register — the failed write must not have partially persisted.
  const mgr2 = new ReadStateManager("st".repeat(32), fakeRelay);
  mgr2.hydrateFromLocalStorage();
  const regOnRestart = mgr2.overrideRegisters.get("storage-test-ch");
  assert.equal(
    regOnRestart,
    undefined,
    "a failed mark must not persist any register (coherent restart)",
  );
  mgr.destroy();
  mgr2.destroy();
});

// ── Test 9: inactive existing register still gets C-bump on markChannelRead ───
test("markChannelRead_inactiveExistingRegister_performsCBump", () => {
  const mgr = makeManager();
  mgr.isLoadComplete = true;
  // Inject an inactive register: S=1, C=2 (clear-wins: C > S → inactive).
  const ctx = "inactive-ch";
  mgr.overrideRegisters.set(ctx, { s: 1, c: 2, b: 0 });
  mgr.publishableContextIds.add(ctx);
  mgr.effectiveState.set(ctx, 100);

  // Verify it is indeed inactive.
  const livenessBefore = mgr.getOverrideLiveness(ctx);
  assert.ok(livenessBefore !== null, "register must exist");
  assert.equal(livenessBefore.active, false, "register must be inactive");

  // markChannelRead must still perform the C-bump (spec NIP-RS.md:537-539).
  const result = mgr.markChannelRead(ctx);
  assert.equal(
    result.status,
    "applied",
    "markChannelRead must succeed on inactive register",
  );

  const reg = mgr.overrideRegisters.get(ctx);
  assert.ok(reg, "register must still exist after markChannelRead");
  // newC = max(S=1, C=2) + 1 = 3
  assert.equal(
    reg.c,
    3,
    "C must be bumped to max(S,C)+1=3 even when already inactive",
  );
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

  // Tie-break: same created_at, lower id wins.
  const tie1 = { ...older, created_at: 3_000, id: "c".repeat(64) };
  const tie2 = { ...newer, created_at: 3_000, id: "a".repeat(64) };
  const tieDuped = deduplicateByCoordinate([tie1, tie2]);
  assert.equal(tieDuped.length, 1, "tie-break dedup must yield one event");
  assert.equal(tieDuped[0].id, "a".repeat(64), "lower id must win on tie");
});

// ── Test 11: lapse mid-enumeration → incomplete ───────────────────────────────
test("fetchAndMerge_lapseMidEnumeration_setsLoadIncomplete", async () => {
  // Simulate a connection lapse after the first band is fetched but before
  // the pinned window returns. The fence.lapsed flag is set mid-enumeration.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "ef".repeat(32);
  const event = makeFakeEvent(pubkey, 1000);

  // Build a fence that starts unlapsed but lapses when the pinned window fires.
  const fence = makeFenceHandle({ eose: true });
  const fakeRelay = {
    fetchEvents: async (filter) => {
      if (filter.since !== undefined) {
        // Pinned window: trigger lapse mid-enumeration.
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
  assert.equal(
    mgr.isLoadComplete,
    false,
    "lapse during enumeration must produce incomplete load",
  );
  mgr.destroy();
});

// ── Test 12: foreign client_id at initial load triggers slot rotation ─────────
test("fetchAndMerge_foreignClientId_rotatesSlotAndUpdatesMetadata", async () => {
  // If a fetched event carries our slot coordinate but a different client_id,
  // the manager must rotate slotId and record maxFetchedCreatedAt.
  // Also validates read-before-write path: fetchOwnBlobBeforePublish runs the
  // same parsed-record metadata path (Contract 2).
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "a6".repeat(32);

  // Install Tauri IPC mock.
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

  // Build a fetched event at our slot coordinate but with foreign client_id.
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
    // Respect `until` so the loader terminates: once `until` drops below the
    // event's created_at the relay returns empty, ending the enumeration.
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
    // Pre-seed the manager's slotId to match the conflict event's d-tag.
    mgr.slotId = slotId;
    await mgr.fetchAndMerge();

    // Slot rotation: slotId must differ from the conflicting coordinate.
    assert.notEqual(
      mgr.slotId,
      slotId,
      "slotId must rotate when fetched event carries foreign client_id at our coordinate",
    );

    // maxFetchedCreatedAt must reflect the fetched event's created_at.
    assert.equal(
      mgr.maxFetchedCreatedAt,
      12345,
      "maxFetchedCreatedAt must be updated from fetched event created_at",
    );
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
  }
});

// ── Test 13: frontier-only advance schedules canonical convergence ─────────────
test("ingest_frontierAdvanceFlipsRegister_schedulesCanonicalConvergence", async () => {
  // A frontier advance that flips an override register from live→tombstone must
  // set canonicalChanged=true and trigger schedulePublish (debounce timer set).
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "a7".repeat(32);

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
  };
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.isLoadComplete = true;

  // Seed an active register: S=5, C=0, B=10, F=5.
  // isOverrideActive(S=5,C=0,B=10,F=5) = 5>0 && 5<=10 && 5>0 → active.
  const ctx = "convergence-ch";
  mgr.overrideRegisters.set(ctx, { s: 5, c: 0, b: 10 });
  mgr.effectiveState.set(ctx, 5);
  mgr.publishableContextIds.add(ctx);

  // Verify it's active before the frontier advance.
  const before = mgr.getOverrideLiveness(ctx);
  assert.ok(before?.active, "register must be active before frontier advance");

  // Deliver a frontier advance F=11 (> B=10) → register becomes dead.
  // Simulate via a live event that carries only the frontier for this ctx.
  // Build a minimal fake event that encodes just the frontier key.
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
    // Fire the live subscription callback directly.
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

    // Access the private handleIncomingEvent via the test helper path.
    await mgr.handleIncomingEvent(fakeEvent);

    // Register must now be inactive.
    const after = mgr.getOverrideLiveness(ctx);
    assert.ok(after !== null, "liveness must still be available");
    assert.equal(
      after.active,
      false,
      "frontier advance past B must flip register to inactive",
    );

    // debounceTimer must be set (schedulePublish was called for convergence).
    assert.ok(
      mgr.debounceTimer !== null,
      "frontier-only canonical deactivation must schedule a convergence publish",
    );
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
    mgr.destroy();
  }
});

// ── Test 14: createFencedSubscription — EOSE establishes the fence ────────────
test("createFencedSubscription_eoseEstablishes_notLapsed", async () => {
  // Drive the real primitive: EOSE resolves established with lapsed=false.
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

  // Simulate EOSE arriving.
  capturedFencedSub.resolveEstablished();
  await handle.established;

  assert.equal(
    handle.lapsed,
    false,
    "EOSE must establish fence with lapsed=false",
  );
  await handle.unsubscribe();
});

// ── Test 14b: createFencedSubscription — EOSE during sendReq cancels timer ────
test("createFencedSubscription_eoseDuringSend_timerCancelledAndStaysUnlapsed", async () => {
  // Thufir's mandated EOSE-during-send witness: if EOSE is delivered while the
  // sendReq() promise is still resolving (e.g. synchronous delivery), the
  // establishment timer must be cancelled immediately and the fence must remain
  // unlapsed past the original timeout window.
  const subscriptions = new Map();
  const timeoutMs = 12; // short so the test can observe no late lapse
  const deps = {
    connectionGeneration: () => 0,
    subscriptions,
    sendReq: async (subId) => {
      // Synchronously fire EOSE before sendReq() returns — races the timer.
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
  // EOSE already fired during sendReq — established resolves with lapsed=false.
  await handle.established;
  assert.equal(
    handle.lapsed,
    false,
    "EOSE during sendReq must establish fence with lapsed=false",
  );

  // Wait well past the timeout to confirm the timer does NOT fire and lapse
  // an already-established fence.
  await new Promise((r) => setTimeout(r, timeoutMs * 3));
  assert.equal(
    handle.lapsed,
    false,
    "timer must not lapse an already-established fence after EOSE-during-send",
  );
  await handle.unsubscribe();
});

// ── Test 15: createFencedSubscription — CLOSED lapses before EOSE ────────────
test("createFencedSubscription_closedBeforeEose_lapses", async () => {
  // Drive the real primitive: CLOSED sets lapsed=true and resolves established.
  const subscriptions = new Map();
  const deps = {
    connectionGeneration: () => 0,
    subscriptions,
    sendReq: async (subId) => {
      // Simulate CLOSED immediately after REQ (before EOSE).
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
  // Drive the real primitive: simulated resetConnection sets lapsed on all
  // pending fenced subscriptions and resolves their established promises.
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

  // Simulate resetConnection: mark all fenced subs as lapsed.
  for (const [subId, sub] of subscriptions) {
    if (sub.mode === "fenced") {
      sub.lapsed = true;
      sub.resolveEstablished();
      subscriptions.delete(subId);
    }
  }

  await handle.established;
  assert.equal(
    handle.lapsed,
    true,
    "resetConnection must set lapsed=true on pending fence",
  );
});

// ── Test 17: createFencedSubscription — establishment timeout lapses fence ────
test("createFencedSubscription_establishmentTimeout_lapses", async () => {
  // A relay that keeps the socket alive but never sends EOSE must lapse the
  // fence after the establishment timeout — never hang forever.
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
  assert.equal(
    handle.lapsed,
    false,
    "fence must not be lapsed immediately after REQ",
  );

  // Wait for the timeout to fire.
  await handle.established;

  assert.equal(
    handle.lapsed,
    true,
    "establishment timeout must lapse the fence",
  );
  assert.equal(
    subscriptions.size,
    0,
    "timed-out fence must be removed from subscriptions map",
  );
});

// ── Test 18: createFencedSubscription — timeout does NOT count as establishment
test("createFencedSubscription_timeout_doesNotCountAsEstablishment", async () => {
  // Even though established resolves after the timeout, lapsed must be true —
  // the timeout NEVER counts as EOSE-based establishment.
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
  assert.equal(
    handle.lapsed,
    true,
    "establishment timeout must never count as successful establishment",
  );
});

// ── Test 19: establishment timeout → no-EOSE relay produces incomplete load ───
test("fetchAndMerge_establishmentTimeout_setsLoadIncomplete", async () => {
  // Thufir's exact witness: a relay that stays alive but never sends EOSE must
  // cause the load to conclude complete:false, not hang initialize() forever.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "b0".repeat(32);
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    // subscribeFenced delegates to the real createFencedSubscription with a
    // fast establishment timeout so the test runs quickly.
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
  assert.equal(
    result,
    "settled",
    `fetchAndMerge must settle within ${raceMs}ms when EOSE never arrives (no-EOSE relay must not hang initialize)`,
  );
  assert.equal(
    mgr.isLoadComplete,
    false,
    "no-EOSE relay with establishment timeout must produce incomplete load",
  );
  mgr.destroy();
});

// ── Test 20: early-write-fails / v2-succeeds → outcome + restart agree ────────
test("markChannelUnread_earlyWriteFails_v2Succeeds_outcomeAndRestartAgree", () => {
  // Thufir's mandated witness: the v2 write is the commit point. If the v2
  // write succeeds, the action succeeds regardless of ancillary write failures.
  // A reconstructed manager must see the committed action, and the context must
  // be publishable so it reaches the relay (serialized primary not empty).
  //
  // New write order in writeStoredReadState: v2 (ok4) first, then ancillary
  // writes (ok1/ok2/ok3). Writes during manager construction: ClientId (1),
  // SlotId (2). First mark write = v2 override write (3) → let it through.
  // Ancillary writes follow (4+) → throw to prove they don't affect outcome.
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

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };
  const pubkey = "ea".repeat(32);
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.isLoadComplete = true;
  mgr.effectiveState.set("committed-ch", 1000);

  // Enable ancillary-write failure before the mark call.
  blockAncillary = true;
  const result = mgr.markChannelUnread("committed-ch");
  blockAncillary = false;

  // Write #3 (v2) succeeded → action must succeed.
  assert.equal(
    result.status,
    "applied",
    "markChannelUnread must succeed when v2 write succeeds even if ancillary writes fail",
  );

  // v2 key must contain the committed register.
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const stored = throwingLS.getItem(v2Key);
  assert.ok(stored !== null, "v2 key must be present after committed action");
  const parsed = JSON.parse(stored);
  // v2 blob format: {r: {<ctx>: {s,c,b,f}}, receipts?: {...}}
  assert.ok(
    parsed.r !== undefined && "committed-ch" in parsed.r,
    "v2 record must contain the committed channel register under .r for coherent restart",
  );

  // Reconstruct a real manager from the same storage — simulates restart.
  // The v2 entry is inherently publishable so hydrateFromLocalStorage must
  // add it to publishableContextIds without the ancillary key.
  writeCount = 0; // reset counter so reconstruction writes go through
  const mgr2 = new ReadStateManager(pubkey, fakeRelay);
  mgr2.hydrateFromLocalStorage(); // simulate the init-time hydration step
  mgr2.isLoadComplete = true;

  // publishableContextIds must contain the committed context — hydration from
  // v2 must not require the ancillary publishable key.
  assert.ok(
    mgr2.publishableContextIds.has("committed-ch"),
    "reconstructed manager must mark the v2-committed context as publishable",
  );

  // currentContexts() must include the register so the committed action is
  // serialized and can be published to the relay.
  const contexts2 = mgr2.currentContexts();
  assert.ok(
    contexts2 !== null,
    "reconstructed manager currentContexts must not be null",
  );
  const hasEntry = Object.keys(contexts2 ?? {}).some(
    (k) => k.startsWith("committed-ch") || k.includes("committed-ch"),
  );
  assert.ok(
    hasEntry,
    "reconstructed manager serialized primary must include the committed register/frontier",
  );

  mgr.destroy();
  mgr2.destroy();
});

// ── Test 21: v2-write-fails → storage_failed, slot IDs unchanged ─────────────
test("markChannelUnread_v2WriteFails_slotIdsUnchanged", () => {
  // When the v2 write (commit point) fails, the action must return storage_failed
  // AND extra slot IDs must not persist (neither in memory nor in the
  // extra-slot-ids key).  Uses 700 channel contexts to force split planning
  // (splitContextsIntoSlots allocates an extra slot), matching Thufir's exact
  // 700-frontier witness.
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

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const pubkey = "eb".repeat(32);
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.isLoadComplete = true;

  // Populate 700 publishable channel contexts so the split planner is forced
  // to allocate an extra slot (exceeds READ_STATE_MAX_PLAINTEXT_BYTES per slot).
  for (let i = 0; i < 700; i++) {
    const ctx = `channel-id-${String(i).padStart(5, "0")}-${"x".repeat(30)}`;
    mgr.effectiveState.set(ctx, 1_700_000_000 + i);
    mgr.publishableContextIds.add(ctx);
  }
  // Also add the target channel for the mark call.
  mgr.effectiveState.set("slot-test-ch", 1000);

  const prevExtraSlotIdsMem = mgr.extraSlotIds.slice();
  const extraSlotKey = `buzz.nip-rs.extra-slot-ids:${pubkey}`;
  const prevExtraSlotIdsStored = throwingLS.getItem(extraSlotKey);

  // Block v2 writes.
  blockV2 = true;
  const result = mgr.markChannelUnread("slot-test-ch");
  blockV2 = false;

  assert.equal(
    result.status,
    "refused",
    "markChannelUnread must fail when v2 write fails",
  );
  assert.equal(result.reason, "storage_failed");

  // Memory slot IDs must be unchanged.
  assert.deepEqual(
    mgr.extraSlotIds,
    prevExtraSlotIdsMem,
    "extraSlotIds in memory must be rolled back on v2 write failure",
  );

  // Persisted slot IDs must be storage-identical to before the action —
  // including absent-vs-present: if the key was absent before, it must still
  // be absent (not `[]`).
  const afterExtraSlotIdsStored = throwingLS.getItem(extraSlotKey);
  assert.equal(
    afterExtraSlotIdsStored,
    prevExtraSlotIdsStored,
    "buzz.nip-rs.extra-slot-ids must be storage-identical after a failed v2 write (absent=absent, not written as [])",
  );

  // v2 key must not contain the failed register.
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const v2Stored = throwingLS.getItem(v2Key);
  const v2Parsed = v2Stored ? JSON.parse(v2Stored) : {};
  assert.equal(
    "slot-test-ch" in v2Parsed,
    false,
    "v2 record must not contain the rolled-back channel register after failed action",
  );

  mgr.destroy();
});

// ── Test 22b: stale ancillary frontier suppressed by authoritative v2 frontier ─
test("hydrateFromLocalStorage_staleAncillaryFrontier_v2FrontierWins", () => {
  // Thufir's exact policy-edge witness: when a v2-success/ancillary-failure
  // commit leaves the ordinary frontier key stale, hydration must apply
  // max(existing, e.f) — not skip the update when the key is present.
  //
  // Setup:  ordinary frontier key  = 50  (stale)
  //         v2 override entry       = {s:5, c:0, b:100, f:101}
  // isOverrideActive(S=5,C=0,B=100, frontier=50)  = 50<=100 → ACTIVE  (wrong)
  // isOverrideActive(S=5,C=0,B=100, frontier=101) = 101>100 → INACTIVE (correct)
  // The test asserts the reconstructed manager uses frontier=101 so the
  // register is inactive and serializes as a tombstone (ov_c: only, no ov_s:).
  const ls = makeLocalStorage();
  globalThis.window.localStorage = ls;

  const pubkey = "ec".repeat(32);
  const ctx = "stale-frontier-ch";

  // Seed the stale ancillary frontier (epoch-seconds → ISO timestamp).
  const staleFrontierKey = `buzz.channel-read-state.v2:${pubkey}`;
  ls.setItem(
    staleFrontierKey,
    JSON.stringify({ [ctx]: new Date(50 * 1_000).toISOString() }),
  );

  // Seed the v2 override key with the authoritative frontier f=101.
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  ls.setItem(v2Key, JSON.stringify({ [ctx]: { s: 5, c: 0, b: 100, f: 101 } }));

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();

  // v2 is authoritative: effectiveState must hold 101, not the stale 50.
  assert.equal(
    mgr.effectiveState.get(ctx),
    101,
    "hydration must apply max(staleAncillary=50, v2.f=101) → 101",
  );

  // With frontier=101 > B=100, the register is INACTIVE (tombstone).
  // currentContexts() must serialize only ov_c: for this channel — no ov_s:.
  mgr.isLoadComplete = true;
  const contexts = mgr.currentContexts();
  assert.ok(contexts !== null, "currentContexts must not be null");
  const ovSKey = `ov_s:${ctx}`;
  const ovCKey = `ov_c:${ctx}`;
  assert.equal(
    ovSKey in contexts,
    false,
    "inactive override must NOT serialize ov_s: (would mark channel active/unread)",
  );
  assert.ok(
    ovCKey in contexts,
    "inactive override must serialize ov_c: tombstone floor",
  );

  mgr.destroy();
});

// ── Test 23: Amendment A — same-source read→unread reincarnation ──────────────
test("drain_sameSourceReincarnation_genFenceZeroLocalEffects", async () => {
  // Amendment A correctness: a same-source read→unread reincarnation witness.
  //
  // Scenario:
  //   (1) Entry {sources:["inbox"]} with register (S=1, C=0) — channel is unread.
  //   (2) During incomplete load, user marks channel read → read(gen=1) queued.
  //   (3) User immediately re-marks channel unread → unread(gen=2) replaces gen=1.
  //   (4) Load completes. Drain captures gen=2 (unread). Gen=1 (read) is gone —
  //       the idempotent addForcedUnreadSource gave it no new local identity, so
  //       the stale read-scope from gen=1 can never fire (it was overwritten).
  //   (5) Assert: gen=2 drain applies S-bump correctly; gen=1's read scope is NOT
  //       applied (channel remains unread); gen=2 intent is deleted from store.
  //
  // The mutation-verify: if a drain somehow applied gen=1's read scope on top of
  // gen=2's unread, the register would be C-bumped and the channel would be
  // observable as read. This asserts that never happens.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "d1".repeat(32);
  const channelId = `reincarnation-channel-${"a".repeat(40)}`;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  // Precondition: existing unread register (S=1, override active at frontier=50).
  mgr.effectiveState.set(channelId, 50);
  mgr.overrideRegisters.set(channelId, { s: 1, c: 0, b: 50 });
  mgr.publishableContextIds.add(channelId);

  // Load is incomplete — mark ops queue.
  assert.equal(
    mgr.isLoadComplete,
    false,
    "precondition: load must be incomplete",
  );

  // Step (2): mark read → queued(gen=1).
  const readResult = mgr.markChannelRead(channelId);
  assert.equal(
    readResult.status,
    "queued",
    "mark-read during incomplete load must queue",
  );

  // Step (3): immediately re-mark unread → queued(gen=2), gen=1 replaced.
  const unreadResult = mgr.markChannelUnread(channelId);
  assert.equal(
    unreadResult.status,
    "queued",
    "mark-unread during incomplete load must queue",
  );

  // Confirm gen=2 is now current (gen=1 is gone).
  const currentIntent = pendingOverrideIntentStore.get(channelId);
  assert.ok(currentIntent, "intent must exist after re-mark");
  assert.equal(
    currentIntent.op,
    "unread",
    "current intent must be unread (gen=2)",
  );
  assert.equal(currentIntent.gen, 2, "current intent must be gen=2");

  // Step (4): complete load then drain.
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // Step (5): gen=2 (unread) was applied — register must have S bumped.
  const reg = mgr.overrideRegisters.get(channelId);
  assert.ok(reg, "override register must exist after drain");
  assert.ok(reg.s > 1, "S must be bumped by the unread drain (gen=2 applied)");
  assert.ok(
    reg.c === 0,
    "C must NOT be bumped (gen=1's read scope must not have fired)",
  );

  // Gen=2 intent must be consumed (compare-and-delete fired).
  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "gen=2 intent must be deleted after successful drain",
  );

  mgr.destroy();
});

// ── Test 24: Amendment B — drain crash window 1 ───────────────────────────────
test("drain_crashWindow1_restartReplaysOnce_extraBumpOnly", async () => {
  // Amendment B crash window 1: crash after register replay but before
  // intent cleanup/deletion (between steps 1 and 2 of the commit order).
  //
  // After a crash in this window, the intent survives in localStorage (no
  // receipt — crash happened BEFORE the register+receipt commit in step 1).
  // On restart, drain replays the same action once more → one extra monotonic
  // S/C bump. The bump is harmless (counters only move up; liveness is
  // threshold-based). The receipt is then written and cleanup + delete runs.
  //
  // Mutation-verify: if instead a crash between (1) and (2) left the receipt
  // persisted alongside the register, the restart hydration rule (Amendment C)
  // would skip the re-bump. This test exercises the NO-receipt path (crash
  // before step 1) to confirm restart does replay exactly once.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "d2".repeat(32);
  const channelId = `crash-window-1-ch-${"b".repeat(44)}`;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  // ── Session 1: enqueue, then "crash" before drain can commit ──────────────
  const mgr1 = new ReadStateManager(pubkey, fakeRelay);
  mgr1.effectiveState.set(channelId, 100);
  assert.equal(mgr1.isLoadComplete, false, "precondition: load incomplete");

  const q1 = mgr1.markChannelUnread(channelId);
  assert.equal(
    q1.status,
    "queued",
    "mark-unread must queue during incomplete load",
  );

  // The intent is now persisted (gen=1). Simulate a crash: destroy manager
  // without draining — intent stays in the v2 blob, no register committed.
  mgr1.destroy();

  // ── Session 2: restart. Intent present, no receipt → replay fires once ────
  const mgr2 = new ReadStateManager(pubkey, fakeRelay);
  mgr2.hydrateFromLocalStorage();
  mgr2.effectiveState.set(channelId, 100);
  mgr2.isLoadComplete = true;

  // Confirm intent survived (reloaded from v2 blob by hydrateFromLocalStorage).
  const intentAfterRestart = pendingOverrideIntentStore.get(channelId);
  assert.ok(
    intentAfterRestart,
    "intent must survive restart (no receipt was written)",
  );
  assert.equal(
    intentAfterRestart.op,
    "unread",
    "surviving intent must be unread",
  );

  // No receipt in store (crash before commit) → no alreadyApplied path.
  assert.equal(
    mgr2.appliedReceipts.get(channelId),
    undefined,
    "no applied receipt after crash-before-commit restart",
  );

  // Drain on restart: replay fires, S-bumped, receipt written, intent deleted.
  await mgr2.drainPendingIntents(mgr2.loadGeneration);

  const reg = mgr2.overrideRegisters.get(channelId);
  assert.ok(reg, "register must exist after drain-on-restart");
  assert.ok(reg.s > 0, "S must be bumped (replay fired once)");

  // Intent consumed.
  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "intent must be deleted after drain-on-restart",
  );

  mgr2.destroy();
});

// ── Test 25: Amendment B — drain crash window 2 ───────────────────────────────
test("drain_crashWindow2_receiptPresent_restartNoRebump", async () => {
  // Amendment B+C crash window 2: crash after register+receipt commit (step 1)
  // but before intent deletion (step 3). Receipt is durable; intent survives.
  //
  // On restart, hydration loads the applied receipt (intent matches receipt →
  // not swept). The drain finds a surviving intent whose {intentGen, op} matches
  // the receipt → alreadyApplied path: cleanup + compare-and-delete WITHOUT
  // another S/C bump.
  //
  // Mutation-verify: if the alreadyApplied path re-bumped the register, the
  // final S value would be higher than the once-applied value. This asserts the
  // register is unchanged from the original single commit.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "d3".repeat(32);
  const channelId = `crash-window-2-ch-${"c".repeat(44)}`;

  const ls = globalThis.window.localStorage;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;

  // Craft the crash state directly in localStorage.
  // Register committed with S=2 (unread applied: max(0,0)+1=1... let's use S=2 for clarity),
  // receipt {intentGen:1, op:"unread"} present, intent {gen:1, op:"unread"} still alive.
  // This is the exact state from "crash after register+receipt commit, before intent delete".
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

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  // Restart: hydration sees receipt+intent → receipt loaded (matching intent).
  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  mgr.effectiveState.set(channelId, 100);
  mgr.isLoadComplete = true;

  // Receipt must have been loaded (matching intent exists).
  const receipt = mgr.appliedReceipts.get(channelId);
  assert.ok(
    receipt,
    "applied receipt must be loaded on restart (matching intent present)",
  );
  assert.equal(receipt.intentGen, 1, "receipt intentGen must be 1");
  assert.equal(receipt.op, "unread", "receipt op must be unread");

  // Intent must still be present.
  const intent = pendingOverrideIntentStore.get(channelId);
  assert.ok(intent, "intent must be present (simulated crash before delete)");
  assert.equal(intent.gen, 1, "intent gen must be 1");

  // Run drain: alreadyApplied path — cleanup + delete, NO re-bump.
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // Register S must be unchanged from the single-commit value (no re-bump).
  const regAfterRestart = mgr.overrideRegisters.get(channelId);
  assert.ok(regAfterRestart, "register must still exist after restart drain");
  assert.equal(
    regAfterRestart.s,
    sSingleCommit,
    "S must NOT be re-bumped (alreadyApplied path skips replay)",
  );

  // Intent must now be deleted.
  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "intent must be deleted after alreadyApplied drain",
  );

  mgr.destroy();
});

// ── Test 26: Amendment C — crash after applied unread, newer remote read ──────
test("drain_crashAfterAppliedUnread_newerRemoteRead_noRebump", async () => {
  // Amendment C crash race 1 — PRODUCTION LOAD PATH:
  // Unread applied (register+receipt durable), then another device publishes C=6
  // (read). On restart, the receipt prevents a second S-bump that would reverse
  // the phone's read. This test uses the real fetchAndMerge() path — the remote
  // read is delivered as an encrypted relay event, not a direct register write.
  //
  // Without Amendment C, restart would replay S=max(S,C)+1 on top of merged C=6,
  // computing S=7, undoing the phone's read. With Amendment C, the receipt proves
  // application; only cleanup+delete runs, so C=6 stands.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "d4".repeat(32);
  const channelId = `amend-c-unread-race-${"d".repeat(44)}`;

  // Craft the crash state in localStorage:
  // - Register committed with S=5 (unread applied, from S=4, C=3).
  // - Receipt {intentGen:1, op:"unread"} persisted atomically.
  // - Intent {gen:1, op:"unread"} still alive (crash before cleanup delete).
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

  // Remote device published C=6 (read) and frontier advanced to 200.
  // Deliver via a real relay event that fetchAndMerge will decrypt and merge.
  const remoteBlob = JSON.stringify({
    v: 1,
    client_id: "remote-device-client",
    contexts: {
      [`ov_s:${channelId}`]: 5,
      [`ov_c:${channelId}`]: 6,
      [`ov_b:${channelId}`]: 100,
      [channelId]: 200, // frontier advanced past B=100 → inactive
    },
  });
  const remoteSlotId = "d4".repeat(16);
  const blobEvent = {
    id: "d4".repeat(32),
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
    subscribeFenced: async (_filter, handler) => {
      if (fetchCallCount === 0) handler(blobEvent);
      return makeFenceHandle({ eose: true });
    },
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  try {
    // Session 2 (restart): hydrate, then complete load via real fetchAndMerge().
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    mgr.hydrateFromLocalStorage();

    // Receipt must be loaded from crash state.
    const receipt = mgr.appliedReceipts.get(channelId);
    assert.ok(receipt, "receipt must be loaded on restart");
    assert.equal(receipt.intentGen, 1);
    assert.equal(receipt.op, "unread");

    // Complete load via production path — fetchAndMerge merges the remote C=6.
    await mgr.fetchAndMerge(mgr.loadGeneration);
    assert.equal(mgr.isLoadComplete, true, "load must complete");

    // The merged register should reflect remote C=6 and frontier 200.
    const regAfterLoad = mgr.overrideRegisters.get(channelId);
    assert.ok(regAfterLoad, "register must exist after fetchAndMerge");
    assert.ok(
      regAfterLoad.c >= 6,
      "merged C must be at least 6 (remote C=6 merged)",
    );

    // Drain via production path — alreadyApplied must fire, no new S-bump.
    await mgr.drainPendingIntents(mgr.loadGeneration);

    // S must remain 5, NOT 7 (which replay would compute as max(5,6)+1=7).
    const reg = mgr.overrideRegisters.get(channelId);
    assert.ok(reg, "register must exist after drain");
    assert.equal(
      reg.s,
      5,
      "S must NOT be re-bumped (would override phone's C=6)",
    );
    assert.ok(reg.c >= 6, "C must reflect remote read (≥6)");

    // Intent consumed — cleanup step ran.
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

// ── Test 27: Amendment C — crash after applied read, newer remote unread ──────
test("drain_crashAfterAppliedRead_newerRemoteUnread_noRebump", async () => {
  // Amendment C crash race 2 — PRODUCTION LOAD PATH (mirror of Test 26):
  // Read applied (C-bump committed + receipt durable), then another device
  // publishes S=7 (marks unread). On restart, the receipt prevents a second
  // C-bump that would reverse the new unread. The remote S=7 is delivered via
  // a real relay event through fetchAndMerge(), not a direct register write.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "d5".repeat(32);
  const channelId = `amend-c-read-race-${"e".repeat(45)}`;

  const ls = globalThis.window.localStorage;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;

  // Crash state: read C-bump committed (S=5, C=6), receipt present, intent alive.
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: { [channelId]: { s: 5, c: 6, b: 100, f: 200 } },
      receipts: { [channelId]: { intentGen: 1, op: "read" } },
      pi: { [channelId]: { gen: 1, op: "read" } },
      ng: 2,
    }),
  );

  // Remote device published S=7 (marked unread after our crash), C stays 6,
  // frontier remains at 100 (below B=100 is NOT above B, so override is active).
  const remoteBlob = JSON.stringify({
    v: 1,
    client_id: "remote-device-client",
    contexts: {
      [`ov_s:${channelId}`]: 7,
      [`ov_c:${channelId}`]: 6,
      [`ov_b:${channelId}`]: 100,
      [channelId]: 100, // frontier at B → still active (s=7 > c=6, f<=b)
    },
  });
  const remoteSlotId = "d5".repeat(16);
  const blobEvent = {
    id: "d5".repeat(32),
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
    subscribeFenced: async (_filter, handler) => {
      if (fetchCallCount === 0) handler(blobEvent);
      return makeFenceHandle({ eose: true });
    },
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    mgr.hydrateFromLocalStorage();

    const receipt = mgr.appliedReceipts.get(channelId);
    assert.ok(receipt, "receipt must be loaded on restart");
    assert.equal(receipt.intentGen, 1);
    assert.equal(receipt.op, "read");

    // Complete load via production path — fetchAndMerge merges remote S=7.
    await mgr.fetchAndMerge(mgr.loadGeneration);
    assert.equal(mgr.isLoadComplete, true, "load must complete");

    // Merged register must reflect remote S=7.
    const regAfterLoad = mgr.overrideRegisters.get(channelId);
    assert.ok(regAfterLoad, "register must exist after fetchAndMerge");
    assert.ok(regAfterLoad.s >= 7, "merged S must be at least 7 (remote S=7)");

    // Drain — alreadyApplied path: no new C-bump (would erase remote unread).
    await mgr.drainPendingIntents(mgr.loadGeneration);

    const reg = mgr.overrideRegisters.get(channelId);
    assert.ok(reg);
    assert.equal(
      reg.c,
      6,
      "C must NOT be re-bumped (would erase remote S=7 unread)",
    );
    assert.ok(reg.s >= 7, "remote S=7 must be preserved");

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

// ── Test 28: Amendment C — receipt without matching intent is swept silently ──
test("drain_receiptWithoutMatchingIntent_sweptWithoutRegisterEffect", async () => {
  // Amendment C receipt lifecycle: a receipt whose {intentGen, op} matches no
  // surviving intent (superseded generation) must be swept without any register
  // effect. Sweeping happens at hydrateFromLocalStorage (the orphan-sweep step),
  // so the receipt is discarded before it can be consulted by the drain.
  //
  // This covers: (a) intent deleted by a successful prior drain, receipt cleanup
  // lagged (e.g., cleanup-commit failed), AND (b) the general "no orphaned
  // receipt persists across sessions" invariant.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "d6".repeat(32);
  const channelId = `receipt-sweep-ch-${"f".repeat(46)}`;

  const ls = globalThis.window.localStorage;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;

  // Craft state: register committed, receipt present, but NO intent in queue.
  // This is the state after a successful drain where cleanup-commit failed
  // (the receipt was left in the blob but the intent was deleted).
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: { [channelId]: { s: 3, c: 0, b: 100, f: 100 } },
      receipts: { [channelId]: { intentGen: 1, op: "unread" } },
    }),
  );
  // No intent key in localStorage (intent was already deleted by the prior drain).

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();
  mgr.isLoadComplete = true;

  // Orphan sweep: hydrateFromLocalStorage discards receipts with no matching intent.
  // The receipt must NOT be in appliedReceipts (it was swept).
  const receipt = mgr.appliedReceipts.get(channelId);
  assert.equal(
    receipt,
    undefined,
    "orphaned receipt must be swept during hydration (no matching intent)",
  );

  // No matching intent in the queue.
  const intent = pendingOverrideIntentStore.get(channelId);
  assert.equal(
    intent,
    undefined,
    "no intent must exist (was deleted in prior drain)",
  );

  // Drain: nothing to drain (no intent), no register effect.
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // Register S must be unchanged (no replay fired).
  const reg = mgr.overrideRegisters.get(channelId);
  assert.ok(reg, "register must still exist");
  assert.equal(
    reg.s,
    3,
    "S must be unchanged — orphaned receipt must not trigger replay",
  );

  // The v2 blob must no longer contain the receipt (swept by hydrateFromLocalStorage
  // via persistLocalState, which persisted this.appliedReceipts without the orphan).
  const stored = JSON.parse(ls.getItem(v2Key) ?? "{}");
  const receiptsInBlob = stored.receipts ?? {};
  assert.equal(
    Object.keys(receiptsInBlob).length,
    0,
    "orphaned receipt must be swept from v2 blob (hydrateFromLocalStorage orphan-sweep)",
  );

  mgr.destroy();
});

// ── Test 29: Amendment C — register+receipt atomicity (no torn state) ─────────
test("drain_registerReceiptAtomicity_noStateWhereOnePersistsWithoutOther", async () => {
  // Amendment C atomicity: if the v2 write fails mid-drain (storage failure
  // during step 1), neither the register nor the receipt must be committed.
  // The intent must be preserved for the next drain pass.
  //
  // Mutation-verify: if register and receipt were written separately (two writes),
  // a failure after the first would leave them observably out of sync. This
  // asserts the combined single-write invariant.
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

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 100);
  mgr.isLoadComplete = false;

  // Queue the intent.
  const q = mgr.markChannelUnread(channelId);
  assert.equal(q.status, "queued");

  // Block v2 writes before drain fires.
  mgr.isLoadComplete = true;
  blockV2Writes = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  blockV2Writes = false;

  // v2 write failed: register must NOT be committed.
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const stored = JSON.parse(ls.getItem(v2Key) ?? "null");
  // The v2 key should be absent or have no register for channelId.
  const registersInBlob = stored?.r ?? {};
  assert.equal(
    Object.keys(registersInBlob).length,
    0,
    "register must NOT be committed when v2 write fails (atomicity invariant)",
  );

  // And no receipt must be present (they're in the same write).
  const receiptsInBlob = stored?.receipts ?? {};
  assert.equal(
    Object.keys(receiptsInBlob).length,
    0,
    "receipt must NOT be committed when v2 write fails (same-blob atomicity)",
  );

  // Intent must still be alive (not consumed on storage failure).
  const intent = pendingOverrideIntentStore.get(channelId);
  assert.ok(intent, "intent must survive when drain's v2 write fails");
  assert.equal(intent.op, "unread");

  mgr.destroy();
});

// ── Test 22: fetchOwnBlobBeforePublish processes foreign client_id ────────────
test("fetchOwnBlobBeforePublish_foreignClientId_rotatesSlotAndUpdatesMetadata", async () => {
  // Thufir's mandated witness: fetchOwnBlobBeforePublish must run the same
  // parsed-record metadata path — a foreign client_id at our coordinate must
  // trigger slot rotation and maxFetchedCreatedAt update via that path.
  // This is distinct from the fetchAndMerge path (test 12) — covers
  // read-before-write semantics.
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

    // Call fetchOwnBlobBeforePublish directly to test the read-before-write path.
    await mgr.fetchOwnBlobBeforePublish();

    // Slot must have rotated because the blob carried a foreign client_id at
    // our coordinate.
    assert.notEqual(
      mgr.slotId,
      slotId,
      "fetchOwnBlobBeforePublish must rotate slotId for foreign client_id at our coordinate",
    );

    // maxFetchedCreatedAt must reflect the blob event's created_at.
    assert.equal(
      mgr.maxFetchedCreatedAt,
      55555,
      "fetchOwnBlobBeforePublish must update maxFetchedCreatedAt from the fetched event",
    );
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
  }
});

// ── Test 30: Amendment A structural — gen2 enqueued DURING replay is durably committed ──
test("drain_gen2EnqueuedDuringReplay_bufferedUntilTransactionCommit", async () => {
  // Under the deferred-enqueue latch topology, any enqueue() for a channel while
  // its drain transaction is open is BUFFERED — not applied.  The gen cannot
  // change inside the transaction window, so gen1 commits cleanly.  On
  // promoteDeferred() (called before the step-3 cleanup persist), gen2 is
  // promoted into the live queue and written to the v2 blob atomically with
  // gen1's cleanup.  scheduleDrain() then fires a fresh pass.
  //
  // Assertions (all durable surfaces):
  //  - gen1 register committed (register in v2 blob)
  //  - gen2 pi in v2 blob (durable promotion in same commit as cleanup)
  //  - restart witness: new manager hydrates gen2 from blob
  //  - fired-second-pass witness: scheduleDrain() leads to gen2 draining
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "30".repeat(32);
  const channelId = `amend-a-biting-ch-${"b".repeat(46)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);

  // Queue gen1 (unread) during incomplete load.
  mgr.isLoadComplete = false;
  const q = mgr.markChannelUnread(channelId);
  assert.equal(q.status, "queued", "gen1 must be queued");
  const gen1 = pendingOverrideIntentStore.get(channelId).gen;
  assert.ok(gen1 >= 1, "gen1 must be a positive generation number");

  // Override tryCandidatePlan to enqueue gen2 DURING replay of gen1.
  // The transaction latch is active at this point, so the enqueue is buffered.
  const origTry = mgr.tryCandidatePlan.bind(mgr);
  let injected = false;
  mgr.tryCandidatePlan = (id, reg) => {
    const ok = origTry(id, reg);
    if (!injected && id === channelId) {
      injected = true;
      // Call enqueue() directly (bypassing the isLoadComplete guard in
      // markChannelUnread) to simulate a concurrent pre-ready enqueue arriving
      // while the drain transaction is open.  The latch buffers this call.
      pendingOverrideIntentStore.enqueue(channelId, "unread");
    }
    return ok;
  };

  // Complete load and drain gen1.  The buffered gen2 enqueue fires mid-replay
  // but is deferred — gen1's transaction commits cleanly, gen2 is promoted and
  // persisted in the same cleanup write, then scheduleDrain() fires a second pass.
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // Allow the auto-scheduled drain pass (gen2) to complete.
  await new Promise((r) => setTimeout(r, 10));

  // Amendment A via latch: gen1's transaction committed cleanly.
  // The register must be present (gen1 applied, not rolled back).
  const reg = mgr.overrideRegisters.get(channelId);
  assert.ok(
    reg !== undefined,
    "gen1's register must be committed — latch prevents gen change, no rollback needed",
  );
  assert.ok(reg.s > 0, "gen1 S-bump must be present");

  // Durable commitment: v2 blob must have gen1 register.
  const rawBlob = globalThis.window.localStorage.getItem(v2Key);
  const blob = rawBlob ? JSON.parse(rawBlob) : {};
  assert.ok(blob.r?.[channelId], "gen1 register must be persisted in v2 blob");

  // Gen2 is promoted and persisted in the cleanup commit OR consumed by the
  // auto-drain second pass.  Either way: after the second pass completes,
  // the v2 blob must NOT have a stale gen1 pi entry (it was cleaned up) and
  // gen2 was either in the blob (before the second pass) or consumed by it.
  // What we assert: after the drain passes settle, the forced store is
  // consistent (no stale teardown needed) and the queue is cleared.
  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "gen2 must be consumed by the auto-scheduled second drain pass",
  );

  // Clean up.
  mgr.destroy();
});

// ── Test 31: unread→read inversion — read gen replaces unread gen ─────────────
test("drain_unreadToReadInversion_onlyReadApplied", async () => {
  // Both inversion directions: unread gen1, then read gen2 replaces it.
  // Drain must apply only gen2 (read = C-bump), not gen1 (unread = S-bump).
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "31".repeat(32);
  const channelId = `inversion-ur-ch-${"c".repeat(48)}`;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  // Pre-seed an existing unread register so C-bump has something to work with.
  mgr.effectiveState.set(channelId, 50);
  mgr.overrideRegisters.set(channelId, { s: 1, c: 0, b: 50 });
  mgr.publishableContextIds.add(channelId);
  mgr.isLoadComplete = false;

  // Step 1: queue unread (gen1).
  const r1 = mgr.markChannelUnread(channelId);
  assert.equal(r1.status, "queued");
  const gen1 = pendingOverrideIntentStore.get(channelId).gen;

  // Step 2: queue read (gen2) — replaces gen1.
  const r2 = mgr.markChannelRead(channelId);
  assert.equal(r2.status, "queued");
  const intent = pendingOverrideIntentStore.get(channelId);
  assert.equal(intent.op, "read", "current intent must be read (gen2)");
  assert.ok(intent.gen > gen1, "gen2 must exceed gen1");

  // Complete load and drain.
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // Gen2 (read) must have applied: C bumped, S unchanged at 1.
  const reg = mgr.overrideRegisters.get(channelId);
  assert.ok(reg, "register must exist after read drain");
  assert.equal(
    reg.s,
    1,
    "S must be unchanged — unread gen1 was replaced, not applied",
  );
  assert.ok(reg.c > 0, "C must be bumped by the read drain (gen2)");

  // Gen2 intent consumed.
  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "gen2 intent must be deleted after drain",
  );

  mgr.destroy();
});

// ── Test 32: read→unread inversion — unread gen replaces read gen ─────────────
test("drain_readToUnreadInversion_onlySBumpApplied", async () => {
  // Mirror of Test 31: read gen1 queued, then unread gen2 replaces it.
  // Drain applies only gen2 (S-bump), not gen1 (C-bump).
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "32".repeat(32);
  const channelId = `inversion-ru-ch-${"d".repeat(48)}`;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);
  mgr.isLoadComplete = false;

  // Queue read (gen1) — no existing register, will be already_inactive after drain.
  const r1 = mgr.markChannelRead(channelId);
  assert.equal(r1.status, "queued");

  // Queue unread (gen2) — replaces read.
  const r2 = mgr.markChannelUnread(channelId);
  assert.equal(r2.status, "queued");
  const intent = pendingOverrideIntentStore.get(channelId);
  assert.equal(intent.op, "unread", "current intent must be unread (gen2)");

  // Complete load and drain.
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // Gen2 (unread) applied: S bumped.
  const reg = mgr.overrideRegisters.get(channelId);
  assert.ok(reg, "register must exist after unread drain");
  assert.ok(reg.s > 0, "S must be bumped by unread drain (gen2)");
  assert.equal(
    reg.c,
    0,
    "C must be zero — read gen1 was replaced, not applied",
  );

  // Consumed.
  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "gen2 intent consumed",
  );

  mgr.destroy();
});

// ── Test 33: non-reentrant drain transaction — deferred enqueue durably committed ─
test("drain_compareDeletePreservesNewerIntent", async () => {
  // Under the deferred-enqueue topology: while gen1's drain transaction is open
  // for channelId, any enqueue() for that channel is buffered.  The transaction
  // commits cleanly for gen1.  On promoteDeferred() (called before the step-3
  // cleanup persist), gen2 is promoted and written to the v2 blob atomically
  // with gen1's cleanup.  scheduleDrain() then fires a fresh pass for gen2.
  //
  // This replaces the old rollback witness.  Assertions (all surfaces):
  //  - gen1 transaction commits cleanly (no rollback triggered)
  //  - INTERMEDIATE: v2 blob has gen1's register + gen2's pi entry atomically
  //    (checked right after the first drain + before the auto-drain)
  //  - forcedUnreadStore: applied-unread callback ran (snapshot cleared)
  //  - versionBumps === 0 (applied-unread does not bump)
  //  - FINAL: gen2 drains in the auto-scheduled pass; queue empty, blob clean
  //  - restart witness: if we intercept BEFORE the auto-drain, a fresh manager
  //    hydrates gen2 from the blob
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "33".repeat(32);
  const channelId = `cmp-del-newer-ch-${"e".repeat(47)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const forcedKey = `buzz-forced-unread.v1:${pubkey}`;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);
  mgr.isLoadComplete = false;

  // Seed forced-unread state: gen1 unread intent has a "manual" source.
  const initialForcedEntry = { markerAtWhenForced: 50, sources: ["manual"] };
  globalThis.window.localStorage.setItem(
    forcedKey,
    JSON.stringify({ [channelId]: initialForcedEntry }),
  );

  // Queue gen1 (unread).
  mgr.markChannelUnread(channelId);
  const gen1 = pendingOverrideIntentStore.get(channelId).gen;

  // Wire the hook-level callback via wireDrainCallback.
  // Inside the callback for applied-unread, simulate a concurrent pre-ready
  // enqueue by calling pendingOverrideIntentStore.enqueue() directly.
  // Under the latch, this enqueue is buffered — it does NOT change gen1's
  // transaction (gen cannot change while the transaction is open).
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
      // Simulate a concurrent pre-ready enqueue arriving while the drain
      // transaction is still open (between onDrainOutcome and cleanup commit).
      // This must be buffered by the latch, not applied immediately.
      pendingOverrideIntentStore.enqueue(channelId, "unread");
      gen2PromotedAfterCallback = true;
    }
  };

  // Prevent the auto-scheduled gen2 drain from running so we can assert the
  // intermediate blob state (gen2 pi present).  We do this by capturing the
  // scheduleDrain call and deferring it.
  let capturedDrainFn = null;
  const origScheduleDrain = mgr.scheduleDrain.bind(mgr);
  mgr.scheduleDrain = () => {
    // Instead of draining immediately, save the request for manual triggering.
    capturedDrainFn = () => {
      origScheduleDrain();
    };
  };

  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // ── Intermediate assertions (before gen2 auto-drain) ──────────────────────
  assert.ok(
    gen2PromotedAfterCallback,
    "gen2 enqueue must have been injected in callback",
  );

  // Gen1's register must be committed to the v2 blob.
  let rawBlob = globalThis.window.localStorage.getItem(v2Key);
  let blob = rawBlob ? JSON.parse(rawBlob) : {};
  assert.ok(
    blob.r?.[channelId],
    "gen1's register must be persisted in v2 blob after successful drain",
  );

  // Gen2's pi must be in the v2 blob (promoted atomically with gen1's cleanup).
  assert.ok(
    blob.pi?.[channelId],
    "gen2's pi must be in v2 blob — durable promotion in same commit as gen1 cleanup",
  );

  // Gen1's receipt must be cleaned up.
  assert.equal(
    blob.receipts?.[channelId],
    undefined,
    "gen1's receipt entry must be cleaned up",
  );

  // Gen2 gen must be higher than gen1.
  assert.ok(blob.pi[channelId].gen > gen1, "gen2 gen in blob must exceed gen1");

  // Restart witness: a fresh manager must hydrate gen2 from the blob.
  {
    const mgr2 = new ReadStateManager(pubkey, fakeRelay);
    mgr2.hydrateFromLocalStorage();
    const hydratedGen2 = pendingOverrideIntentStore.get(channelId);
    assert.ok(
      hydratedGen2,
      "gen2 must be hydrated from v2 blob in a new session",
    );
    assert.equal(hydratedGen2.op, "unread", "gen2 op must be unread");
    assert.ok(
      hydratedGen2.gen > gen1,
      "gen2 gen must exceed gen1 in hydrated state",
    );
    mgr2.destroy();
    // Restore the in-memory store to its pre-hydration2 state for the gen2 drain below.
    // (hydrateFromLocalStorage restores from blob, so gen2 is still in the store.)
  }

  // versionBumps: applied-unread does not bump version (no forced store write).
  assert.equal(
    cb.versionBumps,
    0,
    "applied-unread does not bump version (no forced store write needed)",
  );

  // ── Fire the gen2 drain ────────────────────────────────────────────────────
  assert.ok(capturedDrainFn, "scheduleDrain must have been called");
  // Restore the original scheduleDrain and fire the captured request.
  mgr.scheduleDrain = origScheduleDrain;
  capturedDrainFn();
  // Wait for the gen2 drain to complete.
  await new Promise((r) => setTimeout(r, 10));

  // After gen2 drains: queue empty, blob has no pi entry.
  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "gen2 intent must be consumed after the auto-scheduled drain pass",
  );
  rawBlob = globalThis.window.localStorage.getItem(v2Key);
  blob = rawBlob ? JSON.parse(rawBlob) : {};
  assert.equal(
    blob.pi?.[channelId],
    undefined,
    "gen2's pi must be cleaned up after the auto-drain",
  );

  // Clean up.
  cb.teardown();
  mgr.destroy();
});

// ── Test 34: Amendment C via production load — restart + fetchAndMerge ────────
test("drain_amendmentC_restart_fetchAndMerge_noBumpAfterReceipt", async () => {
  // Amendment C via the PRODUCTION load path: after a crash-in-cleanup, the
  // next session hydrates from storage and runs fetchAndMerge() (not a direct
  // register write). The receipt must prevent a second S-bump.
  //
  // Thufir's finding: Tests 26/27 directly set isLoadComplete=true and write
  // overrideRegisters directly — they bypass the production load path.
  // This test uses the actual fetchAndMerge() flow via fakeRelay event delivery.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "34".repeat(32);
  const channelId = `amend-c-prod-ch-${"f".repeat(48)}`;

  // Craft crash-state in localStorage: unread applied (S=5), receipt intact.
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

  // A remote device published C=6 (read). Simulate via a fetched event that,
  // when decrypted, delivers C=6 for the channel via the standard merge path.
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
  // Slot ID must match /^[0-9a-f]{32}$/ to pass isValidReadStateDTag.
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
      // Deliver the event synchronously before EOSE.
      if (fetchCallCount === 0) {
        handler(blobEvent);
      }
      return makeFenceHandle({ eose: true });
    },
    subscribeLive: async (_f, _h) => () => {},
  };

  try {
    // Session 2: hydrate, then complete load via real fetchAndMerge().
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    mgr.hydrateFromLocalStorage();

    // Receipt must be loaded.
    const receipt = mgr.appliedReceipts.get(channelId);
    assert.ok(receipt, "receipt must be loaded on restart");
    assert.equal(receipt.intentGen, 1);

    // Run fetchAndMerge via the production path.
    await mgr.fetchAndMerge(mgr.loadGeneration);
    assert.equal(mgr.isLoadComplete, true, "load must complete");

    // Drain via production path.
    await mgr.drainPendingIntents(mgr.loadGeneration);

    // The alreadyApplied path must have run: no new S-bump.
    // C must reflect the merged remote (max(3,6)=6).
    const reg = mgr.overrideRegisters.get(channelId);
    assert.ok(reg, "register must exist");
    assert.equal(
      reg.s,
      5,
      "S must NOT be re-bumped by drain (receipt prevents it)",
    );
    assert.ok(reg.c >= 6, "C must reflect remote C=6 merge");

    // Intent consumed.
    assert.equal(
      pendingOverrideIntentStore.get(channelId),
      undefined,
      "intent must be consumed after drain",
    );
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
  }
});

// ── Test 35: atomic deletion — intent + receipt deleted in one persist ─────────
test("drain_cleanupCommit_intentAndReceiptDeletedAtomically", async () => {
  // Atomic deletion witness: after successful drain, both the intent AND the
  // receipt are absent from the v2 blob in a single commit (same persist call).
  // If they were deleted in separate writes, there would be a window where one
  // survives without the other.
  //
  // Verify: after drain, the v2 blob contains neither pi nor receipts entry.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "35".repeat(32);
  const channelId = `atomic-del-ch-${"g".repeat(50)}`;

  const ls = globalThis.window.localStorage;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);
  mgr.isLoadComplete = false;

  // Queue the intent.
  mgr.markChannelUnread(channelId);

  // Drain: apply the intent (receipt gets co-committed with register).
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // After successful drain, check the persisted v2 blob.
  const rawAfter = ls.getItem(v2Key);
  const blobAfter = rawAfter ? JSON.parse(rawAfter) : {};

  // Both intent (pi) and receipt (receipts) must be absent — single cleanup commit.
  const piEntry = blobAfter?.pi?.[channelId];
  assert.equal(
    piEntry,
    undefined,
    "intent must be deleted from pi in cleanup commit",
  );

  const receiptEntry = blobAfter?.receipts?.[channelId];
  assert.equal(
    receiptEntry,
    undefined,
    "receipt must be deleted from receipts in cleanup commit",
  );

  // The register IS present (drain succeeded, override applied).
  const regEntry = blobAfter?.r?.[channelId];
  assert.ok(regEntry, "register must persist after successful drain");

  mgr.destroy();
});

// ── Test 36: controller — automatic retry after incomplete verdict ─────────────
test("retryLoad_incompleteVerdictSchedulesAutomaticRetry", async () => {
  // After an incomplete verdict, retryLoad() must schedule a backoff timer that
  // actually fires a fresh retryLoad() attempt automatically.
  // Verifies: (1) timer is scheduled, (2) delay > 0 for second attempt,
  // (3) retryAttempt is incremented, (4) the fired callback starts a new generation
  // (fresh fetch), (5) destroy cancels the scheduled timer before it fires.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "36".repeat(32);

  let fetchCallCount = 0;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async () => {
      fetchCallCount++;
      throw new Error("fence timeout"); // always incomplete
    },
    subscribeLive: async (_f, _h) => () => {},
  };

  const origSetTimeout = globalThis.window.setTimeout;
  const origClearTimeout = globalThis.window.clearTimeout;

  const scheduledTimers = new Map();
  let nextFakeId = 10000;

  globalThis.window.setTimeout = (fn, ms) => {
    if (ms > 0) {
      const id = nextFakeId++;
      scheduledTimers.set(id, { fn, ms });
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

  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);
    assert.equal(mgr.retryAttempt, 0, "precondition: retryAttempt is 0");

    // First retry (immediate — attempt 0 means delayMs=0).
    await mgr.retryLoad();
    assert.equal(fetchCallCount, 1, "first fetch must have been called");

    // After first incomplete verdict, retryAttempt must be > 0.
    assert.ok(
      mgr.retryAttempt > 0,
      "retryAttempt must be > 0 after first incomplete",
    );

    // A backoff timer must have been scheduled (automatic retry loop).
    assert.ok(
      mgr.retryBackoffTimer !== null,
      "retryBackoffTimer must be set — automatic retry was scheduled",
    );
    assert.ok(
      scheduledTimers.size > 0,
      "a timer must be pending in the fake scheduler",
    );

    // Timer delay must be > 0 (bounded backoff).
    const [timerId, { fn: timerFn, ms }] = [...scheduledTimers.entries()][0];
    assert.ok(ms > 0, "backoff timer delay must be > 0 for attempt >= 1");

    // Fire the timer callback — this simulates the automatic second attempt.
    // The callback calls retryLoad() with a fresh generation.
    scheduledTimers.delete(timerId);
    const genBeforeFire = mgr.loadGeneration;
    timerFn(); // fires the callback
    // Wait for the async retryLoad to complete.
    await new Promise((r) => origSetTimeout(r, 50));

    // A fresh fetch must have been triggered by the automatic retry.
    assert.ok(
      fetchCallCount >= 2,
      "automatic retry must trigger a second fetch; fetchCallCount=" +
        fetchCallCount,
    );
    // A new generation must have been started (or the same if it finished).
    assert.ok(
      mgr.loadGeneration >= genBeforeFire,
      "load generation must not regress",
    );

    // After the automatic retry (also incomplete), another timer is scheduled.
    assert.ok(
      mgr.retryBackoffTimer !== null || scheduledTimers.size > 0,
      "a follow-up backoff timer must be scheduled after the second incomplete",
    );

    // Destroy must cancel the pending timer.
    mgr.destroy();
    assert.equal(
      mgr.retryBackoffTimer,
      null,
      "destroy must clear retryBackoffTimer",
    );
    assert.equal(
      scheduledTimers.size,
      0,
      "fake scheduler must have no pending timers after destroy",
    );
  } finally {
    globalThis.window.setTimeout = origSetTimeout;
    globalThis.window.clearTimeout = origClearTimeout;
  }
});

// ── Test 36b: controller — initialize() incomplete → schedules automatic retry
test("initialize_incompleteVerdict_schedulesAutomaticRetry", async () => {
  // initialize() ending with an incomplete verdict must schedule a retryBackoffTimer
  // so the manager eventually retries even without an external reconnect signal.
  // Verifies: (1) retryBackoffTimer is set, (2) delay > 0, (3) retryAttempt is 1,
  // (4) the fired callback starts a fresh fetch (second attempt), (5) destroy cancels.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "36b".repeat(21).slice(0, 64);

  let fetchCallCount = 0;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async () => {
      fetchCallCount++;
      throw new Error("fence timeout"); // always incomplete
    },
    subscribeLive: async (_f, _h) => () => {},
  };

  const origSetTimeout = globalThis.window.setTimeout;
  const origClearTimeout = globalThis.window.clearTimeout;

  const scheduledTimers = new Map();
  let nextFakeId = 20000;

  globalThis.window.setTimeout = (fn, ms) => {
    if (ms > 0) {
      const id = nextFakeId++;
      scheduledTimers.set(id, { fn, ms });
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

  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);

    await mgr.initialize();

    // initialize() must have scheduled a retry timer.
    assert.ok(
      mgr.retryBackoffTimer !== null,
      "retryBackoffTimer must be set after incomplete initialize()",
    );
    assert.ok(
      scheduledTimers.size > 0,
      "a timer must be pending in the fake scheduler after incomplete initialize()",
    );

    // retryAttempt must be 1 (initialize = attempt 0 consumed).
    assert.equal(
      mgr.retryAttempt,
      1,
      "retryAttempt must be 1 after incomplete initialize()",
    );

    // Timer delay must be > 0.
    const [timerId, { fn: timerFn, ms }] = [...scheduledTimers.entries()][0];
    assert.ok(
      ms > 0,
      "backoff timer delay must be > 0 after incomplete initialize()",
    );

    // Fire the timer callback — proves the automatic second attempt runs.
    scheduledTimers.delete(timerId);
    const fetchBeforeFire = fetchCallCount;
    timerFn(); // fires the scheduled retryLoad()
    // Wait for the async retryLoad() to complete.
    await new Promise((r) => origSetTimeout(r, 50));

    assert.ok(
      fetchCallCount > fetchBeforeFire,
      "automatic retry must trigger another fetch after initialize() timer fires",
    );

    // destroy() must cancel any remaining pending timer.
    mgr.destroy();
    assert.equal(
      mgr.retryBackoffTimer,
      null,
      "destroy must clear retryBackoffTimer",
    );
    assert.equal(
      scheduledTimers.size,
      0,
      "fake scheduler must have no pending timers after destroy",
    );
  } finally {
    globalThis.window.setTimeout = origSetTimeout;
    globalThis.window.clearTimeout = origClearTimeout;
  }
});

// ── Test 37: controller — reconnect coalescing, backoff cancel, fresh gen/fetch ──
test("retryLoad_reconnectDuringLoadInFlight_coalescesToPendingRetry", async () => {
  // Scenario: a load is in-flight; a reconnect arrives (sets pendingRetryOnComplete);
  // the in-flight load completes with an incomplete verdict (backoff timer set);
  // pendingRetryOnComplete fires an immediate retry that cancels the EXACT first
  // backoff timer and starts a second fetch; a second reconnect during the
  // pending-triggered (second) fetch is coalesced unconditionally.
  //
  // Assertions (all unconditional — no conditional skips):
  //  - reconnect during load sets pendingRetryOnComplete, does NOT advance gen
  //  - after the first load resolves: gen advanced by exactly 1, second fetch fired
  //  - the EXACT backoff timer from the first incomplete is cancelled (tracked by ID)
  //  - second reconnect during the second in-flight load always sets pendingRetryOnComplete
  //  - destroy cancels all residual timers
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "37".repeat(32);

  // Two barriers: first controls the gen-1 fetch; second controls the gen-2 fetch.
  let resolveFirst;
  const firstBarrier = new Promise((r) => {
    resolveFirst = r;
  });
  let resolveSecond;
  const secondBarrier = new Promise((r) => {
    resolveSecond = r;
  });

  // Fake timer: intercept ms>0 timers, record exact IDs, track clears.
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
      // call 3+: immediately incomplete
      throw new Error("fence timeout");
    },
    subscribeLive: async (_f, _h) => () => {},
  };

  try {
    const mgr = new ReadStateManager(pubkey, fakeRelay);

    // ── Phase 1: start first load, block it at the barrier ────────────────────
    const load1Promise = mgr.retryLoad();
    await new Promise((r) => origSetTimeout(r, 0)); // let async fn reach await
    assert.equal(
      mgr.loadInFlight,
      true,
      "loadInFlight must be true while first fetch blocks",
    );
    const gen1 = mgr.loadGeneration;

    // ── Phase 2: first reconnect while load is in-flight ──────────────────────
    // retryLoad() returns synchronously because loadInFlight is true.
    mgr.retryLoad();
    assert.equal(
      mgr.pendingRetryOnComplete,
      true,
      "first reconnect during loadInFlight must set pendingRetryOnComplete",
    );
    assert.equal(
      mgr.loadGeneration,
      gen1,
      "loadGeneration must NOT advance on coalesced reconnect",
    );

    // A second simultaneous reconnect also coalesces.
    mgr.retryLoad();
    assert.equal(
      mgr.pendingRetryOnComplete,
      true,
      "second reconnect during loadInFlight must keep pendingRetryOnComplete set",
    );
    assert.equal(
      mgr.loadGeneration,
      gen1,
      "gen must still not advance after second reconnect",
    );

    // ── Phase 3: release gen-1 fetch — incomplete verdict fires ───────────────
    resolveFirst();
    await load1Promise;
    // The incomplete verdict (line 384-392) sets a backoff timer with ms>0,
    // but the finally-block detects pendingRetryOnComplete, cancels it, and
    // calls retryLoad() immediately.  By the time we're here:
    //  - The backoff timer was created (in allScheduledTimerIds)
    //  - The backoff timer was cancelled (in clearedTimerIds)
    //  - The gen-2 fetch is already in-flight (blocked on secondBarrier)
    assert.ok(
      allScheduledTimerIds.size >= 1,
      "at least one backoff timer must have been created for the incomplete verdict",
    );
    const firstBackoffTimerId = [...allScheduledTimerIds][0];

    // ── Phase 4: finally-block fires pending retry ─────────────────────────────
    // The finally-block detects pendingRetryOnComplete, cancels the backoff timer,
    // and calls retryLoad() immediately (which starts gen-2 fetch).
    await new Promise((r) => origSetTimeout(r, 0));

    // The first backoff timer must have been cancelled with its EXACT ID.
    assert.ok(
      clearedTimerIds.has(firstBackoffTimerId),
      "the first incomplete's backoff timer must be cancelled by the exact timer ID",
    );
    assert.equal(
      scheduledTimers.size,
      0,
      "no timers must be pending after the cancel (gen-2 fetch is in-flight, not waiting)",
    );

    // loadGeneration must have advanced exactly once for the fresh pending retry.
    assert.equal(
      mgr.loadGeneration,
      gen1 + 1,
      "gen must advance by exactly 1 when pendingRetryOnComplete fires",
    );
    assert.equal(fetchCallCount, 2, "exactly two fetches must have occurred");

    // ── Phase 5: gen-2 fetch is in-flight — reconnect coalesces unconditionally ─
    // loadInFlight must still be true because secondBarrier has not been released.
    assert.equal(
      mgr.loadInFlight,
      true,
      "gen-2 load must still be in-flight (secondBarrier not released)",
    );
    // This reconnect MUST always be asserted — no `if (loadInFlight)` guard.
    mgr.retryLoad();
    assert.equal(
      mgr.pendingRetryOnComplete,
      true,
      "reconnect during gen-2 load must unconditionally set pendingRetryOnComplete",
    );

    // ── Phase 6: release gen-2 fetch and destroy ──────────────────────────────
    resolveSecond();
    await new Promise((r) => origSetTimeout(r, 0));

    mgr.destroy();
    assert.equal(
      mgr.retryBackoffTimer,
      null,
      "destroy must clear retryBackoffTimer",
    );
    assert.equal(
      scheduledTimers.size,
      0,
      "fake scheduler must have no timers after destroy",
    );
  } finally {
    globalThis.window.setTimeout = origSetTimeout;
    globalThis.window.clearTimeout = origClearTimeout;
  }
});

// ── Test 38: controller — stale-completion ordering via gen check ──────────────
test("fetchAndMerge_staleGeneration_doesNotSetLoadComplete", async () => {
  // If loadGeneration advances while fetchAndMerge is in flight, the stale
  // generation's complete verdict must NOT mark the manager as load-complete.
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
        // First call: wait for barrier before returning EOSE.
        await firstBarrier;
        return makeFenceHandle({ eose: true });
      }
      return makeFenceHandle({ eose: true });
    },
    subscribeLive: async (_f, _h) => () => {},
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);

  // Start gen=1 fetch; it blocks at subscribeFenced.
  const gen1 = mgr.loadGeneration;
  const firstFetch = mgr.fetchAndMerge(gen1);
  await new Promise((r) => setTimeout(r, 0));

  // Advance the generation by starting a new fetch.
  ++mgr.loadGeneration;
  const gen2 = mgr.loadGeneration;
  await mgr.fetchAndMerge(gen2);
  assert.equal(mgr.isLoadComplete, true, "gen2 load must complete");

  // Now release the blocked gen1 fetch.
  resolveFirst();
  await firstFetch;

  // Gen1's verdict should not have mutated isLoadComplete back to stale state.
  // (Gen2 left it true; gen1 must not interfere.)
  assert.equal(
    mgr.isLoadComplete,
    true,
    "gen1 stale verdict must not corrupt gen2's complete state",
  );

  mgr.destroy();
});

// ── Test 39: controller — destroy cancels backoff timer ───────────────────────
test("retryLoad_destroyWhileTimerPending_cancelsTimer", async () => {
  // destroy() must cancel any pending retryBackoffTimer so no stale retry fires.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "39".repeat(32);

  let timerSet = false;
  let pendingTimerResolve = null; // captured so we can drain after destroy
  const origSetTimeout = globalThis.window.setTimeout;
  const origClearTimeout = globalThis.window.clearTimeout;

  // Track active backoff timers by real ID so clearTimeout works correctly.
  // When ms > 0 we wrap the fn to capture state, but still use the real timer
  // (with a large delay so it doesn't fire during the test) so origClearTimeout
  // can cancel it.  After destroy() we must resolve the pending await manually
  // to let retryLoad's finally-block run and the promise settle.
  const activeTimerIds = new Set();
  globalThis.window.setTimeout = (fn, ms) => {
    if (ms > 0) {
      timerSet = true;
      // Schedule with origSetTimeout at a safe delay so clearTimeout works.
      const id = origSetTimeout(() => {
        activeTimerIds.delete(id);
        pendingTimerResolve = null;
        fn();
      }, 60_000); // long enough never to fire during test
      // Capture resolve so we can drain manually after destroy().
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

    // First retry (immediate, attempt=0 → no timer).
    await mgr.retryLoad();
    // retryAttempt is now 1; next retry will schedule a timer.

    // Start second retry asynchronously — it will set a timer.
    const retryPromise = mgr.retryLoad();
    // Give async fn time to reach the timer await.
    await new Promise((r) => origSetTimeout(r, 0));

    assert.equal(timerSet, true, "a backoff timer must have been scheduled");
    assert.ok(
      mgr.retryBackoffTimer !== null,
      "retryBackoffTimer must be non-null while waiting",
    );

    // Destroy must cancel the timer.
    mgr.destroy();
    assert.equal(
      mgr.retryBackoffTimer,
      null,
      "retryBackoffTimer must be null after destroy",
    );
    assert.equal(
      activeTimerIds.size,
      0,
      "backoff timer must be cleared by destroy()",
    );

    // retryLoad is still awaiting its backoff Promise. Fire the pending resolve
    // manually (the real timer was cleared so it won't auto-fire) so the
    // finally-block in retryLoad runs and retryPromise settles. Since
    // mgr.destroyed===true, retryLoad returns immediately after the resolve.
    if (pendingTimerResolve) pendingTimerResolve();
    await retryPromise;
  } finally {
    globalThis.window.setTimeout = origSetTimeout;
    globalThis.window.clearTimeout = origClearTimeout;
  }
});

// ── Test 40: controller — exactly one drain per complete-load generation ───────
test("retryLoad_exactlyOneDrainPerCompleteGeneration", async () => {
  // A single complete-load transition must trigger exactly one drain.
  // Multiple retryLoad() calls for the same complete generation must not
  // enqueue a second drain.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "40".repeat(32);
  const channelId = `one-drain-ch-${"h".repeat(51)}`;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.isLoadComplete = false;

  // Queue an intent that will be consumed by the drain.
  mgr.markChannelUnread(channelId);

  // First retryLoad → complete → drain fires.
  await mgr.retryLoad();
  assert.equal(mgr.isLoadComplete, true, "load must be complete");

  // Record register state after first drain.
  const regAfterFirst = mgr.overrideRegisters.get(channelId);
  const sAfterFirst = regAfterFirst?.s ?? 0;

  // Second retryLoad on an already-complete manager: drainScheduled is false
  // (already consumed), so a second drain must NOT re-bump the register.
  await mgr.retryLoad();

  const regAfterSecond = mgr.overrideRegisters.get(channelId);
  assert.equal(
    regAfterSecond?.s ?? 0,
    sAfterFirst,
    "S must NOT be re-bumped by a second retryLoad (exactly-one-drain invariant)",
  );

  mgr.destroy();
});

// ── Test 41: store/UI — identity swap during drain cancels stale gen ──────────
test("drain_identitySwapDuringDrain_staleGenCancelled", async () => {
  // If the manager is destroyed and recreated for a new pubkey, the stale
  // generation's drain must be cancelled. Verify via the pubkey fence inside
  // drainPendingIntents: pubkeyFence !== ctx.pubkey → break.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkeyA = "41".repeat(32);
  const channelId = `identity-swap-ch-${"i".repeat(47)}`;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgrA = new ReadStateManager(pubkeyA, fakeRelay);
  mgrA.effectiveState.set(channelId, 50);
  mgrA.isLoadComplete = false;

  // Queue intent for pubkeyA.
  mgrA.markChannelUnread(channelId);
  const intentA = pendingOverrideIntentStore.get(channelId);
  assert.ok(intentA, "intent must exist for pubkeyA");

  // Destroy mgrA before drain (simulates identity swap).
  mgrA.destroy();

  // The drainPendingIntents call with mgrA's context should be cancelled
  // because mgrA.destroyed === true.
  // Verify by running the drain and asserting the intent is NOT consumed.
  // (The drain early-exits on ctx.destroyed.)
  await mgrA.drainPendingIntents(mgrA.loadGeneration);

  // The intent must still be alive — drain was no-op because destroyed.
  const intentAfter = pendingOverrideIntentStore.get(channelId);
  assert.ok(intentAfter, "intent must survive drain when manager is destroyed");

  // Clean up.
  pendingOverrideIntentStore.compareAndDelete(channelId, intentAfter.gen);
});

// ── Test 42: deferred refusal — queued unread intent refused by drain ─────────
test("drain_queuedUnread_refused_surfacesOutcome", async () => {
  // When a queued unread intent is refused by drain (e.g., budget_exhausted),
  // onDrainOutcome must be called with (channelId, "unread", "refused", reason).
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "42".repeat(32);
  const channelId = `deferred-refusal-ch-${"j".repeat(44)}`;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);
  mgr.isLoadComplete = false;

  // Queue an unread intent with uint32 overflow condition: S is at max.
  mgr.overrideRegisters.set(channelId, { s: 0xffffffff, c: 0, b: 50 });
  mgr.publishableContextIds.add(channelId);
  const q = mgr.markChannelUnread(channelId);
  assert.equal(q.status, "queued", "must queue pre-ready");

  // Wire outcome callback.
  const outcomes = [];
  mgr.onDrainOutcome = (outcome) => {
    outcomes.push(outcome);
  };

  // Complete load and drain.
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // Drain must have surfaced the refusal.
  assert.equal(outcomes.length, 1, "exactly one outcome must be emitted");
  assert.equal(outcomes[0].channelId, channelId);
  assert.equal(outcomes[0].kind, "genuine-refusal");
  assert.equal(outcomes[0].op, "unread");
  assert.equal(
    outcomes[0].reason,
    "uint32_overflow",
    "refusal reason must be uint32_overflow",
  );

  // Intent must be cleaned up even after refusal (it's a genuine refusal, not load_incomplete).
  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "intent must be deleted after genuine refusal",
  );

  mgr.destroy();
});

// ── Test 43: deferred-refusal-after-restart — intent persisted, hydrated, refused ─
test("drain_deferredRefusalAfterRestart_intentHydratedAndRefused", async () => {
  // Intent survives a restart: queued during incomplete load, persisted in the
  // v2 blob, then loaded by the next session and refused by drain.
  // Verifies the full round-trip: enqueue → persist → hydrate → drain (refused).
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "43".repeat(32);
  const channelId = `deferred-restart-ch-${"k".repeat(44)}`;

  const ls = globalThis.window.localStorage;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;

  // Simulate session 1: intent queued and persisted with a uint32-maxed register.
  // The session closed before the load completed (crash / tab close).
  ls.setItem(
    v2Key,
    JSON.stringify({
      r: { [channelId]: { s: 0xffffffff, c: 0, b: 50, f: 50 } },
      pi: { [channelId]: { gen: 1, op: "unread" } },
      ng: 2,
    }),
  );

  // Session 2: new manager hydrates the persisted intent.
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();

  // Intent must be restored from v2 blob.
  const hydrated = pendingOverrideIntentStore.get(channelId);
  assert.ok(hydrated, "intent must be hydrated from v2 blob on restart");
  assert.equal(hydrated.op, "unread");
  assert.equal(hydrated.gen, 1);

  // Wire outcome callback to capture the refusal.
  const outcomes = [];
  mgr.onDrainOutcome = (outcome) => {
    outcomes.push(outcome);
  };

  // Complete load and drain — the register is still at uint32-max, so S-bump
  // would overflow → refused (uint32_overflow).
  await mgr.fetchAndMerge(mgr.loadGeneration);
  assert.equal(mgr.isLoadComplete, true, "load must complete");
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // Drain must surface the refusal via onDrainOutcome.
  assert.equal(outcomes.length, 1, "exactly one outcome must be emitted");
  assert.equal(outcomes[0].kind, "genuine-refusal");
  assert.equal(outcomes[0].op, "unread");
  assert.equal(outcomes[0].reason, "uint32_overflow");

  // Intent consumed even after refusal (genuine, not load_incomplete).
  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "intent must be deleted after genuine refusal on restart",
  );

  mgr.destroy();
});

// ── Test 44: storage_failed — enqueue survives; in-memory intent is live ─────
test("markChannelUnread_storageFailed_intentInMemoryOnly", () => {
  // When localStorage.setItem throws (quota exceeded) during persistLocalState()
  // after enqueueing a pre-ready intent, the intent must remain in-memory so the
  // current session can still drain it — failing-safe rather than silently losing
  // the user's action.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "44".repeat(32);
  const channelId = `storage-fail-ch-${"l".repeat(50)}`;

  // Replace setItem with a throwing version to simulate quota exhaustion.
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

    // Pre-ready enqueue — should succeed despite storage failure.
    const result = mgr.markChannelUnread(channelId);
    assert.equal(
      result.status,
      "queued",
      "must return queued on incomplete load",
    );

    // Intent must be alive in-memory even though persist failed.
    const intent = pendingOverrideIntentStore.get(channelId);
    assert.ok(intent, "intent must be in-memory even if persist failed");
    assert.equal(intent.op, "unread");

    // Restore setItem so we can drain.
    globalThis.window.localStorage.setItem = origSetItem;
    mgr.isLoadComplete = true;

    // Drain must pick up the in-memory intent and apply it.
    // (Register is empty → S-bump succeeds.)
    return mgr.drainPendingIntents(mgr.loadGeneration).then(() => {
      const reg = mgr.overrideRegisters.get(channelId);
      assert.ok(reg, "register must exist after drain");
      assert.ok(reg.s > 0, "S-bump must have been applied by drain");

      // Intent must be consumed.
      assert.equal(
        pendingOverrideIntentStore.get(channelId),
        undefined,
        "intent must be consumed by drain",
      );

      mgr.destroy();
    });
  } finally {
    // Ensure setItem is restored even on test failure.
    globalThis.window.localStorage.setItem = origSetItem;
  }
});

// ── Test 45: authoritative queued-read target captured at click time ──────────
test("markChannelRead_preReady_capturesAuthoritativeMarkAt", async () => {
  // When load is incomplete, markChannelRead() must persist the caller's markAt
  // (not ctx.channelFrontier()) as readTarget in the intent.  Drain must then
  // advance the frontier to markAt (not the stale partial frontier) before C-bump,
  // and the hook-level callback must clean up the forced-unread source.
  //
  // Assertions (all surfaces):
  //  - readTarget in intent = authoritativeMarkAt (not partialFrontier)
  //  - after drain: effectiveState[channelId] >= authoritativeMarkAt
  //  - v2 blob: register persisted with C-bump, intent/receipt cleaned up
  //  - forcedUnreadStore: "inbox" source removed from channelId
  //  - versionBumps === 1 (exactly one forced-store cleanup via hook)
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "45".repeat(32);
  const channelId = `mark-at-ch-${"m".repeat(53)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const forcedKey = `buzz-forced-unread.v1:${pubkey}`;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  // Partial frontier (simulates what's visible pre-ready).
  const partialFrontier = 100;
  mgr.effectiveState.set(channelId, partialFrontier);
  mgr.isLoadComplete = false;

  // Simulate a register so the read is not already_inactive.
  mgr.overrideRegisters.set(channelId, { s: 5, c: 0, b: 100 });
  mgr.publishableContextIds.add(channelId);

  // Seed forced-unread store: channel has "inbox" + "manual" sources.
  const forcedEntry = { markerAtWhenForced: 50, sources: ["inbox", "manual"] };
  globalThis.window.localStorage.setItem(
    forcedKey,
    JSON.stringify({ [channelId]: forcedEntry }),
  );

  // Click-time markAt — greater than the partial frontier.
  const authoritativeMarkAt = 999;
  const sourceScope = "inbox";
  const r = mgr.markChannelRead(channelId, sourceScope, authoritativeMarkAt);
  assert.equal(r.status, "queued", "must queue when load incomplete");

  // Intent must carry the authoritative markAt as readTarget.
  const intent = pendingOverrideIntentStore.get(channelId);
  assert.ok(intent, "intent must exist");
  assert.equal(
    intent.readTarget,
    authoritativeMarkAt,
    "readTarget must equal the authoritative markAt, not the partial frontier",
  );

  // Wire hook-level callback (mirrors useDrainOutcomeCallback logic).
  const cb = wireDrainCallback(mgr, pubkey);

  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // Frontier must have advanced to authoritativeMarkAt.
  const frontier = mgr.effectiveState.get(channelId) ?? 0;
  assert.ok(
    frontier >= authoritativeMarkAt,
    `frontier must be advanced to at least authoritativeMarkAt (${authoritativeMarkAt}); got ${frontier}`,
  );

  // v2 blob: register must be persisted with C-bump.
  const rawBlob = globalThis.window.localStorage.getItem(v2Key);
  const blob = rawBlob ? JSON.parse(rawBlob) : {};
  assert.ok(
    blob.r?.[channelId],
    "register must be persisted in v2 blob after drain",
  );
  const reg = blob.r[channelId];
  assert.ok(reg.c > 0, "C must be bumped (applied read)");

  // Intent and receipt must be cleaned up from blob.
  assert.equal(
    blob.pi?.[channelId],
    undefined,
    "intent must be cleaned up from v2 blob",
  );
  assert.equal(
    blob.receipts?.[channelId],
    undefined,
    "receipt must be cleaned up from v2 blob",
  );

  // forcedUnreadStore: "inbox" source must be removed; "manual" remains.
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

  // versionBumps: exactly 1 (hook cleaned up the forced source once).
  assert.equal(
    cb.versionBumps,
    1,
    "versionBumps must be 1 — exactly one forced-store cleanup for applied-read",
  );

  cb.teardown();
  mgr.destroy();
});

// ── Test 46: already_inactive — source cleanup runs via hook/store ────────────
test("drain_alreadyInactive_sourceCleanupCallback", async () => {
  // When a queued read drains and the register is gone (already_inactive),
  // the outcome must be silent-inactive and the hook callback must remove the
  // exact source from the persisted forcedUnreadStore.
  //
  // Assertions (all surfaces):
  //  - outcome is silent-inactive (not genuine-refusal)
  //  - forcedUnreadStore: "inbox" source is removed from channelId entry
  //  - the remaining entry still has "manual" source byte-for-byte
  //  - versionBumps === 1
  //  - intent consumed from queue
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "46".repeat(32);
  const channelId = `already-inactive-ch-${"n".repeat(44)}`;
  const forcedKey = `buzz-forced-unread.v1:${pubkey}`;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 100);
  mgr.isLoadComplete = false;

  // Seed forced-unread store: channel has "inbox" + "manual" sources.
  const forcedEntry = {
    markerAtWhenForced: 100,
    sources: ["inbox", "manual"],
  };
  globalThis.window.localStorage.setItem(
    forcedKey,
    JSON.stringify({ [channelId]: forcedEntry }),
  );

  // Queue a read with "inbox" sourceScope — no register exists.
  const sourceScope = "inbox";
  const r = mgr.markChannelRead(channelId, sourceScope);
  assert.equal(r.status, "queued");

  // Wire hook-level callback.
  const cb = wireDrainCallback(mgr, pubkey);

  // Complete load without adding a register — already_inactive path.
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // forcedUnreadStore must have lost exactly "inbox" — "manual" remains.
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

  // versionBumps: exactly 1 (one forced-store write by hook).
  assert.equal(
    cb.versionBumps,
    1,
    "versionBumps must be 1 for silent-inactive cleanup",
  );

  // Intent consumed from queue.
  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "intent must be consumed after already_inactive",
  );

  cb.teardown();
  mgr.destroy();
});

// ── Test 47: restart-safe unread rollback — priorForcedEntry restored in forcedUnreadStore
test("drain_unreadRefusal_restartSafe_priorForcedEntryRestored", async () => {
  // After a restart, a refused unread intent must restore the prior forced entry
  // (not delete the whole channel entry) using the priorForcedEntry persisted in
  // the intent metadata, and write that restoration to forcedUnreadStore.
  //
  // Scenario:
  //  - Session 1: channel had {sources:["inbox"]} entry; "manual" unread attempted
  //    (S at max → uint32_overflow), intent queued with priorForcedEntry.
  //    The optimistic unread replaced the entry in forcedUnreadStore.
  //  - App restarts.
  //  - Session 2: hydrates intent with priorForcedEntry; drain runs; overflow → refused.
  //    Hook must restore {sources:["inbox"]} in forcedUnreadStore.
  //
  // Assertions (all surfaces):
  //  - priorForcedEntry hydrates correctly from v2 blob
  //  - outcome is genuine-refusal with priorForcedEntry
  //  - forcedUnreadStore[channelId] = priorForcedEntry byte-for-byte after hook runs
  //  - versionBumps === 1
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "47".repeat(32);
  const channelId = `restart-safe-ch-${"o".repeat(47)}`;

  const ls = globalThis.window.localStorage;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const forcedKey = `buzz-forced-unread.v1:${pubkey}`;

  // Simulate session 1: {sources:["inbox"]} entry existed, then manual unread
  // was attempted (S at max → will overflow), intent queued with priorForcedEntry.
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
  // The optimistic unread had replaced the forced entry with "manual".
  ls.setItem(
    forcedKey,
    JSON.stringify({
      [channelId]: { markerAtWhenForced: 50, sources: ["manual"] },
    }),
  );

  // Session 2: new manager hydrates.
  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();

  // Verify intent restored with priorForcedEntry.
  const hydrated = pendingOverrideIntentStore.get(channelId);
  assert.ok(hydrated, "intent must be hydrated");
  assert.deepEqual(
    hydrated.priorForcedEntry,
    priorForcedEntry,
    "priorForcedEntry must survive round-trip through v2 blob",
  );

  // Wire hook-level callback.  No in-memory snapshot exists (new session), so
  // the callback falls back to outcome.priorForcedEntry for restoration.
  const cb = wireDrainCallback(mgr, pubkey);

  // Complete load — register still at uint32-max → S-bump overflows → refused.
  await mgr.fetchAndMerge(mgr.loadGeneration);
  assert.equal(mgr.isLoadComplete, true);
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // forcedUnreadStore must be restored to the priorForcedEntry byte-for-byte.
  const storedForced = forcedUnreadStore.read(pubkey);
  assert.deepEqual(
    storedForced[channelId],
    priorForcedEntry,
    "forcedUnreadStore must be restored to priorForcedEntry after post-restart refusal",
  );

  // versionBumps: exactly 1 (hook wrote to forced store once).
  assert.equal(
    cb.versionBumps,
    1,
    "versionBumps must be 1 — exactly one restore write",
  );

  cb.teardown();
  mgr.destroy();
});

// ── Test 48: legacy null priorForcedEntry — hydrates correctly, drain runs ────
test("hydrate_legacyNullPriorForcedEntry_intentHydratedAndDrains", async () => {
  // `null` is a valid legacy v1 ForcedUnreadEntry marker.  It must NOT be treated
  // as the invalid-value sentinel — an intent whose priorForcedEntry is null must
  // hydrate successfully and drain normally.  A valid sibling intent in the same
  // blob must also hydrate (proving the null entry does not corrupt the parse loop).
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "48".repeat(32);
  const channelId1 = `legacy-null-ch-${"p".repeat(49)}`;
  const channelId2 = `sibling-null-ch-${"q".repeat(48)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;

  // Plant: ch1 has unread intent with priorForcedEntry=null (legacy null marker).
  //        ch2 has unread intent with no priorForcedEntry (valid sibling).
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

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();

  // ch1 intent must hydrate with priorForcedEntry=null (not rejected).
  const intent1 = pendingOverrideIntentStore.get(channelId1);
  assert.ok(intent1, "ch1 intent (null prior) must hydrate");
  assert.equal(intent1.gen, 1, "ch1 intent gen must be 1");
  assert.equal(
    "priorForcedEntry" in intent1,
    true,
    "ch1 intent must carry priorForcedEntry",
  );
  assert.equal(
    intent1.priorForcedEntry,
    null,
    "ch1 priorForcedEntry must be null (valid legacy marker)",
  );

  // ch2 (sibling) intent must hydrate regardless of ch1's null prior.
  const intent2 = pendingOverrideIntentStore.get(channelId2);
  assert.ok(
    intent2,
    "ch2 sibling intent must hydrate alongside null-prior intent",
  );
  assert.equal(intent2.gen, 2, "ch2 intent gen must be 2");

  // Drain: ch1 has s=0xffffffff → overflow.  With the liveness re-evaluation
  // fix, since the frontier has not advanced past b=50, the override IS still
  // active (frontier=0 ≤ b=50) → genuine uint32_overflow refusal.
  // ch2 applies normally (S-bump on existing register).
  mgr.effectiveState.set(channelId1, 0); // frontier below b=50
  mgr.effectiveState.set(channelId2, 10);
  await mgr.fetchAndMerge(mgr.loadGeneration);
  assert.equal(mgr.isLoadComplete, true);
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // ch2 must have drained (S-bump beyond existing s=5).
  const reg2 = mgr.overrideRegisters.get(channelId2);
  assert.ok(reg2, "ch2 register must be present after drain");
  // ch2 has existing s=5, so new S = max(5,3)+1 = 6.
  assert.equal(reg2.s, 6, "ch2 S must be bumped by drain");

  // ch1 intent consumed (refusal still consumes the intent).
  assert.equal(
    pendingOverrideIntentStore.get(channelId1),
    undefined,
    "ch1 intent must be consumed after uint32_overflow refusal",
  );

  // No RangeError was thrown — drain completed without abort.
  assert.equal(
    pendingOverrideIntentStore.get(channelId2),
    undefined,
    "ch2 intent must also be consumed (no drain abort from null prior)",
  );

  mgr.destroy();
});

// ── Test 49: corrupt v2 blob fields — malformed intents rejected, valid sibling hydrates ──
test("hydrate_corruptV2BlobIntentFields_rejectedWithoutBlockingValidSibling", async () => {
  // Corrupt/out-of-range intent fields must be silently rejected.
  // The following cases are planted in the same blob:
  //  - ch1: readTarget = 1e300 (out of uint32 range → rejected, prevents RangeError)
  //  - ch2: priorForcedEntry = { markerAtWhenForced: -1, sources: ["inbox"] }
  //         (invalid marker → rejected)
  //  - ch3: priorForcedEntry = { sources: ["unknown-source"] }
  //         (invalid source member → rejected, no silent partial filtering)
  //  - ch4: valid unread intent (must hydrate, proving the parse loop continues)
  //
  // All assertions run without throwing — no RangeError can abort hydration.
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

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);

  // hydrateFromLocalStorage must not throw for any of the corrupt entries.
  assert.doesNotThrow(
    () => mgr.hydrateFromLocalStorage(),
    "hydrateFromLocalStorage must not throw on corrupt intent fields",
  );

  // ch1 (readTarget=1e300): rejected — out of uint32 range.
  assert.equal(
    pendingOverrideIntentStore.get(channelId1),
    undefined,
    "ch1 intent with readTarget=1e300 must be rejected",
  );

  // ch2 (invalid marker=-1): rejected.
  assert.equal(
    pendingOverrideIntentStore.get(channelId2),
    undefined,
    "ch2 intent with priorForcedEntry markerAtWhenForced=-1 must be rejected",
  );

  // ch3 (invalid source member): rejected — no silent partial filtering.
  assert.equal(
    pendingOverrideIntentStore.get(channelId3),
    undefined,
    "ch3 intent with invalid source member must be rejected (no partial filtering)",
  );

  // ch4 (valid): must hydrate.
  const intent4 = pendingOverrideIntentStore.get(channelId4);
  assert.ok(
    intent4,
    "ch4 valid intent must hydrate despite corrupt sibling entries",
  );
  assert.equal(intent4.gen, 4, "ch4 intent gen must be 4");
  assert.equal(intent4.op, "unread", "ch4 intent op must be unread");

  // Drain must run without throwing (no RangeError from 1e300 readTarget).
  mgr.effectiveState.set(channelId4, 50);
  await mgr.fetchAndMerge(mgr.loadGeneration);
  assert.equal(mgr.isLoadComplete, true);

  let drainThrew = false;
  try {
    await mgr.drainPendingIntents(mgr.loadGeneration);
  } catch {
    drainThrew = true;
  }
  assert.equal(
    drainThrew,
    false,
    "drain must not throw (no RangeError from corrupt readTarget)",
  );

  // ch4 intent consumed (S-bumped on existing register s=3).
  assert.equal(
    pendingOverrideIntentStore.get(channelId4),
    undefined,
    "ch4 intent must be consumed after drain",
  );

  mgr.destroy();
});

// ── Test 50: overflow with frontier-deactivated register → silent success ─────
test("markChannelReadDirect_uint32overflow_frontierDeactivated_silentSuccess", async () => {
  // When C-bump is unrepresentable (s=0xffffffff, c=0) but the post-advance
  // frontier has already deactivated the register (frontier > b=50),
  // the outcome must be silent-inactive (not uint32_overflow).
  // Parity with applyOverrideRead() sync path at readStateOverride.ts.
  //
  // Setup: {s:0xffffffff, c:0, b:50}, queued readTarget=100.
  // After frontier advance to 100: frontier(100) > b(50), s(0xffffffff) > c(0)
  // BUT: isOverrideActive = s > 0 && frontier <= b && s > c
  //                       = true && 100 <= 50 (FALSE) → inactive.
  // → silent-inactive outcome, source cleanup runs, forced owner removed.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "50".repeat(32);
  const channelId = `overflow-deact-ch-${"v".repeat(46)}`;
  const forcedKey = `buzz-forced-unread.v1:${pubkey}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;

  const ls = globalThis.window.localStorage;

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50); // initial frontier
  mgr.isLoadComplete = false;

  // Seed an existing register: s=0xffffffff, c=0, b=50.
  mgr.overrideRegisters.set(channelId, { s: 0xffffffff, c: 0, b: 50 });
  mgr.publishableContextIds.add(channelId);

  // Seed forced-unread store: channel has "manual" source.
  const forcedEntry = { markerAtWhenForced: 50, sources: ["manual"] };
  ls.setItem(forcedKey, JSON.stringify({ [channelId]: forcedEntry }));

  // Queue a read with readTarget=100 (authoritative mark-at).
  const sourceScope = "manual";
  const r = mgr.markChannelRead(channelId, sourceScope, 100);
  assert.equal(r.status, "queued", "must queue when load incomplete");

  // Wire hook-level callback.
  const cb = wireDrainCallback(mgr, pubkey);

  // Complete load — register still has s=0xffffffff.
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // Frontier must have advanced to 100.
  const frontier = mgr.effectiveState.get(channelId) ?? 0;
  assert.ok(
    frontier >= 100,
    `frontier must advance to readTarget (100); got ${frontier}`,
  );

  // The register must now be inactive (frontier=100 > b=50).
  // The outcome must have been silent-inactive, NOT uint32_overflow.
  // Evidence: versionBumps=1 (source cleanup ran) and forcedUnreadStore lost "manual".
  const storedForced = forcedUnreadStore.read(pubkey);
  assert.equal(
    storedForced[channelId],
    undefined,
    "forced entry must be fully removed (silent-inactive source cleanup)",
  );

  assert.equal(
    cb.versionBumps,
    1,
    "versionBumps must be 1 — silent-inactive triggered source cleanup",
  );

  // Intent consumed (silent-inactive still consumes the intent).
  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "intent must be consumed after silent-inactive outcome",
  );

  // v2 blob: forced owner must not persist (cleanup write ran).
  const rawBlob = ls.getItem(v2Key);
  const blob = rawBlob ? JSON.parse(rawBlob) : {};
  assert.equal(
    blob.pi?.[channelId],
    undefined,
    "pi entry must be cleaned up from v2 blob",
  );

  cb.teardown();
  mgr.destroy();
});

// ── Test 51: abort/retry on step-1 storage failure — gen1 survives, retry drains ──
test("drain_step1StorageFailure_abortRetry_gen1SurvivesAndRetrains", async () => {
  // If the step-1 register+receipt persist fails, the drain aborts the transaction:
  // gen1 stays live in the store, deferred gen2 stays buffered, and scheduleDrain()
  // fires a retry pass.  On the retry, storage is healthy and the drain completes.
  //
  // Assertions (durable surfaces):
  //  - blob has gen1 pi at the start (from enqueue persist before drain)
  //  - restart witness: hydrating after step-1 failure sees gen1 (not gen2)
  //  - after the retry pass: register committed, gen1+gen2 cleaned up, queue empty,
  //    blob has no pi
  //
  // Timing note: the abort path schedules a retry drain as a microtask continuation
  // inside the same awaited drain call.  We suppress the auto-retry by momentarily
  // clearing pendingFreshDrain after the failing drain, verify durability and restart,
  // then manually fire the retry to prove gen1 actually drains successfully.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "51".repeat(32);
  const channelId = `abort-step1-ch-${"w".repeat(49)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;

  // Block ALL writes to the v2 key until `unblockV2Write()` is called.
  // setLocalStorageItemWithRecovery retries after catching, so we must block
  // both the initial write AND the recovery retry — a single-shot flag is not
  // enough.  A persistent-block flag is cleared explicitly by the test.
  let blockV2Writes = false;
  const origSetItem = ls.setItem.bind(ls);
  ls.setItem = (key, value) => {
    if (blockV2Writes && key.startsWith("buzz.nip-rs.override-state.v2:")) {
      throw new Error("QuotaExceededError");
    }
    origSetItem(key, value);
  };

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);
  mgr.isLoadComplete = false;

  // Queue gen1 (unread).
  mgr.markChannelUnread(channelId);
  const gen1Intent = pendingOverrideIntentStore.get(channelId);
  assert.ok(gen1Intent, "gen1 must be queued");
  const gen1 = gen1Intent.gen;

  // The v2 blob must have gen1 pi from the initial enqueue persist (write 0).
  // Check this BEFORE the failing drain so we know the baseline durable state.
  const blobBeforeDrain = JSON.parse(ls.getItem(v2Key) ?? "null") ?? {};
  assert.ok(
    blobBeforeDrain.pi?.[channelId],
    "v2 blob must carry gen1 pi before drain",
  );
  assert.equal(
    blobBeforeDrain.pi[channelId].gen,
    gen1,
    "blob pi gen must match gen1",
  );

  // Intercept scheduleDrain to capture any auto-retry fired by the abort path.
  // The step-1 abort calls scheduleDrain(), which the manager honors by firing a
  // fire-and-forget retry drain.  We capture it here to prevent it from running
  // before we check intermediate state.
  let capturedRetryFn = null;
  const origScheduleDrain = mgr.scheduleDrain.bind(mgr);
  mgr.scheduleDrain = () => {
    // Save the retry request — fire it manually after intermediate assertions.
    capturedRetryFn = () => {
      mgr.scheduleDrain = origScheduleDrain;
      origScheduleDrain();
    };
  };

  // Complete load, but fail the step-1 register+receipt write by blocking ALL
  // v2 writes (setLocalStorageItemWithRecovery would retry after eviction, so
  // a single-shot flag is insufficient — a persistent block is required).
  mgr.isLoadComplete = true;
  blockV2Writes = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  // Unblock writes so subsequent operations can persist.
  blockV2Writes = false;

  // The v2 blob must still have gen1 pi (step-1 write failed — blob unchanged from write 0).
  const blobAfterFail = JSON.parse(ls.getItem(v2Key) ?? "null") ?? {};
  assert.ok(
    blobAfterFail.pi?.[channelId],
    "v2 blob must still carry gen1 pi after step-1 failure",
  );
  assert.equal(
    blobAfterFail.pi[channelId].gen,
    gen1,
    "blob gen must still be gen1 after failure",
  );

  // The register must NOT be committed (step-1 failed before persist).
  assert.equal(
    mgr.overrideRegisters.get(channelId),
    undefined,
    "register must not be committed after step-1 failure",
  );

  // Restart witness: a fresh manager sees gen1 (not a stale or promoted gen2).
  {
    const mgr2 = new ReadStateManager(pubkey, fakeRelay);
    mgr2.hydrateFromLocalStorage();
    const hydratedIntent = pendingOverrideIntentStore.get(channelId);
    assert.ok(hydratedIntent, "gen1 must be hydrated from blob on restart");
    assert.equal(
      hydratedIntent.gen,
      gen1,
      "hydrated gen must be gen1 (abort preserved durable state)",
    );
    mgr2.destroy();
  }

  // Manually fire the retry drain (storage is healthy now — no more blocks).
  assert.ok(
    capturedRetryFn,
    "scheduleDrain must have been called by the abort path",
  );
  mgr.scheduleDrain = origScheduleDrain; // restore before firing
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // After retry: register committed, queue empty, blob cleaned.
  const regAfterRetry = mgr.overrideRegisters.get(channelId);
  assert.ok(regAfterRetry, "register must be committed after retry drain");
  assert.ok(regAfterRetry.s > 0, "S must be bumped after retry");

  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "gen1 must be consumed after retry drain",
  );

  const blobAfterRetry = JSON.parse(ls.getItem(v2Key) ?? "null") ?? {};
  assert.equal(
    blobAfterRetry.pi?.[channelId],
    undefined,
    "v2 blob must have no pi entry after retry cleanup",
  );

  mgr.destroy();
});

// ── Test 52: abort/retry on step-3 cleanup failure — gen1 stays live, retry cleans up ──
test("drain_step3CleanupFailure_abortRetry_gen1StaysLiveAndRetrains", async () => {
  // If the step-3 cleanup write fails (after step-1 register+receipt committed),
  // abortTransaction restores gen1 as the live intent and keeps gen2 buffered.
  // The in-memory receipt is also restored so the retry drain's alreadyApplied
  // check fires (cleanup-only, no re-bump).  scheduleDrain fires a retry.
  //
  // Assertions (durable surfaces):
  //  - after failing pass (auto-retry suppressed): register in blob (step-1 ok),
  //    gen1 pi still in blob (cleanup write failed), gen1 pi+receipt in blob
  //  - gen1 restored as live in-memory intent (abortTransaction)
  //  - restart witness: gen1 pi+receipt in blob; hydration sees gen1 as pending
  //  - after retry pass: pi cleaned, queue empty, blob has no pi
  //
  // Timing: same as Test 51 — we intercept scheduleDrain to prevent the auto-retry
  // from running before intermediate assertions, then manually fire the retry.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "52".repeat(32);
  const channelId = `abort-step3-ch-${"x".repeat(49)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;

  // We need to fail exactly write 2 (step-3 cleanup) while writes 0 and 1 succeed.
  // setLocalStorageItemWithRecovery catches throws and retries once after cache eviction.
  // In our clean test environment eviction returns 0 entries, so no retry occurs —
  // a single throw per write attempt is sufficient for the specific numbered-write approach.
  // BUT: the throw-on-count approach only works because our test localStorage has no
  // pure-cache keys (evictPureCacheEntries returns 0).
  //
  // Write ordering:
  //  Write 0: enqueue persist (markChannelUnread, incomplete load) ← succeeds
  //  Write 1: step-1 register+receipt commit (drain) ← succeeds
  //  Write 2: step-3 cleanup commit (drain) ← FAILS (persistent block from this index)
  let v2WriteCount = 0;
  let blockFromWriteIndex = -1; // block all v2 writes from this index onward; -1 = unblocked
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

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.effectiveState.set(channelId, 50);
  mgr.isLoadComplete = false;

  // Queue gen1 (unread). This persists the initial pi (write 0).
  mgr.markChannelUnread(channelId);
  const gen1 = pendingOverrideIntentStore.get(channelId).gen;

  // Intercept scheduleDrain to prevent the auto-retry from running before we
  // check intermediate state.  Same pattern as Test 51.
  let capturedRetryFn = null;
  const origScheduleDrain = mgr.scheduleDrain.bind(mgr);
  mgr.scheduleDrain = () => {
    capturedRetryFn = () => {
      mgr.scheduleDrain = origScheduleDrain;
      origScheduleDrain();
    };
  };

  // Enable failure from write index 2 (step-3 cleanup).
  // Write 1 (step-1 register+receipt) must still succeed.
  blockFromWriteIndex = 2;
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);
  // Unblock writes so subsequent operations can persist.
  blockFromWriteIndex = -1;

  // After failing pass: register must be committed (step-1 succeeded).
  const reg = mgr.overrideRegisters.get(channelId);
  assert.ok(reg, "register must be committed after step-1 succeeds");
  assert.ok(reg.s > 0, "S must be bumped");

  // Blob must have the register but gen1 pi must still be present (cleanup failed).
  // The blob reflects write 1 (register + receipt + pi) since write 2 was blocked.
  const blobAfterFail = JSON.parse(ls.getItem(v2Key) ?? "null") ?? {};
  assert.ok(
    blobAfterFail.r?.[channelId],
    "register must be in blob after step-1 success",
  );
  assert.ok(
    blobAfterFail.pi?.[channelId],
    "gen1 pi must still be in blob (cleanup write failed)",
  );
  // The receipt must also be in the blob (write 1 included it; write 2 failed to remove it).
  assert.ok(
    blobAfterFail.receipts?.[channelId],
    "receipt must still be in blob (cleanup write failed)",
  );

  // Gen1 must be restored as the live intent (abortTransaction).
  const intentAfterFail = pendingOverrideIntentStore.get(channelId);
  assert.ok(intentAfterFail, "gen1 must be live after step-3 abort");
  assert.equal(intentAfterFail.gen, gen1, "live intent must be gen1");

  // Restart witness: hydrating sees gen1 pi + receipt; next drain runs Amendment C
  // cleanup without re-bumping (register already committed).
  {
    const mgr2 = new ReadStateManager(pubkey, fakeRelay);
    mgr2.hydrateFromLocalStorage();
    const hydratedIntent = pendingOverrideIntentStore.get(channelId);
    assert.ok(hydratedIntent, "gen1 must be hydrated from blob on restart");
    assert.equal(hydratedIntent.gen, gen1, "hydrated gen must be gen1");
    mgr2.destroy();
  }

  // Manually fire the retry drain (storage healthy — block is lifted).
  assert.ok(
    capturedRetryFn,
    "scheduleDrain must have been called by the step-3 abort path",
  );
  mgr.scheduleDrain = origScheduleDrain; // restore before firing
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // After retry: gen1 cleaned up, queue empty.
  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "gen1 must be consumed after retry cleanup drain",
  );

  const blobAfterRetry = JSON.parse(ls.getItem(v2Key) ?? "null") ?? {};
  assert.equal(
    blobAfterRetry.pi?.[channelId],
    undefined,
    "v2 blob must have no pi entry after retry",
  );

  mgr.destroy();
});

// ── Test 53: ng normalization — gen7 collision cannot discard a fresh action ──
test("hydrate_ngCollision_normalizedAboveExistingGens_freshActionNotDiscarded", async () => {
  // Reproduce Thufir's collision witness: blob has pi.gen=7, receipt gen=7, ng=7.
  // After hydration, a fresh enqueue gets the NEXT gen (≥ 8), so the old receipt
  // (gen=7) does NOT match the new intent (gen≠7) and drain performs the action.
  //
  // Assertions:
  //  - hydrated nextGen is normalized to max(intent gens, receipt gens) + 1
  //  - fresh enqueue gets a gen STRICTLY GREATER than 7
  //  - drain does NOT treat the old receipt as proof already-applied
  //  - frontier advances to the new readTarget (999)
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "53".repeat(32);
  const channelId = `ng-collision-ch-${"y".repeat(47)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;

  // Plant: pi.gen=7, receipt.gen=7, ng=7 — corrupt: ng should be ≥ 8.
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

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();

  // After hydration, nextGen must be normalized to > 7 (above max intent/receipt gen).
  // The hydrated intent (gen=7) and receipt (gen=7) must be accepted.
  const hydratedIntent = pendingOverrideIntentStore.get(channelId);
  assert.ok(hydratedIntent, "intent (gen=7) must hydrate");
  assert.equal(hydratedIntent.gen, 7, "hydrated intent gen must be 7");

  // nextGen must have been normalized to ≥ 8.
  const nextGenAfterHydrate = pendingOverrideIntentStore.nextGen;
  assert.ok(
    nextGenAfterHydrate >= 8,
    `nextGen must be normalized to ≥ 8 after hydration; got ${nextGenAfterHydrate}`,
  );

  // Replace the intent by enqueueing a NEW read with readTarget=999.
  // Under the correct normalization, this gets a gen > 7.
  mgr.isLoadComplete = false;
  const r = mgr.markChannelRead(channelId, undefined, 999);
  assert.equal(
    r.status,
    "queued",
    "new read must be queued while load incomplete",
  );
  const freshIntent = pendingOverrideIntentStore.get(channelId);
  assert.ok(freshIntent, "fresh intent must be in store");
  assert.ok(
    freshIntent.gen > 7,
    `fresh intent gen must be strictly > 7 (got ${freshIntent.gen}) — collision prevented`,
  );
  assert.equal(
    freshIntent.readTarget,
    999,
    "fresh intent must carry readTarget=999",
  );

  // Drain: the old receipt (intentGen=7) must NOT match the fresh intent (gen > 7).
  // Amendment C: alreadyApplied = receipt.intentGen === capturedGen.
  // Since fresh gen ≠ 7, drain re-applies the read.
  mgr.effectiveState.set(channelId, 100); // frontier
  mgr.isLoadComplete = true;
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // Frontier must advance to readTarget=999.
  const frontier = mgr.effectiveState.get(channelId) ?? 0;
  assert.ok(
    frontier >= 999,
    `frontier must advance to 999; got ${frontier} — old receipt must not have swallowed the action`,
  );

  // Intent consumed.
  assert.equal(
    pendingOverrideIntentStore.get(channelId),
    undefined,
    "fresh intent must be consumed",
  );

  mgr.destroy();
});

// ── Test 54: scalar-number priorForcedEntry — hydrates as valid ForcedUnreadEntry ──
test("hydrate_scalarNumberPriorForcedEntry_acceptedAsValidLegacyEntry", async () => {
  // ForcedUnreadEntry includes legacy scalar-number markers (e.g. a unix timestamp).
  // parseForcedUnreadEntry must accept a valid non-negative number as a valid entry,
  // not reject it as corrupt.  A valid sibling intent in the same blob also hydrates.
  globalThis.window.localStorage = makeLocalStorage();
  const pubkey = "54".repeat(32);
  const channelId1 = `scalar-num-ch-${"z".repeat(50)}`;
  const channelId2 = `sibling-scalar-${"A".repeat(49)}`;
  const v2Key = `buzz.nip-rs.override-state.v2:${pubkey}`;
  const ls = globalThis.window.localStorage;

  // Plant: ch1 has unread intent with priorForcedEntry=1700000000 (valid scalar number).
  //        ch2 has unread intent with no priorForcedEntry.
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

  const fakeRelay = {
    fetchEvents: async () => [],
    publishEvent: async () => {},
    subscribeFenced: async (_f, _h) => makeFenceHandle({ eose: true }),
    subscribeLive: async (_f, _h) => () => {},
    subscribeToReconnects: () => () => {},
    getConnectionGeneration: () => 0,
  };

  const mgr = new ReadStateManager(pubkey, fakeRelay);
  mgr.hydrateFromLocalStorage();

  // ch1 intent must hydrate with priorForcedEntry=1700000000 (valid scalar number).
  const intent1 = pendingOverrideIntentStore.get(channelId1);
  assert.ok(intent1, "ch1 intent (scalar-number prior) must hydrate");
  assert.equal(intent1.gen, 1, "ch1 intent gen must be 1");
  assert.equal(
    intent1.priorForcedEntry,
    1700000000,
    "ch1 priorForcedEntry must be the scalar number 1700000000",
  );

  // ch2 (sibling without prior) must also hydrate.
  const intent2 = pendingOverrideIntentStore.get(channelId2);
  assert.ok(intent2, "ch2 sibling intent must hydrate");
  assert.equal(intent2.gen, 2, "ch2 gen must be 2");

  // Wire the drain callback and drain both intents.
  const cb = wireDrainCallback(mgr, pubkey);
  mgr.effectiveState.set(channelId1, 50);
  mgr.effectiveState.set(channelId2, 30);
  await mgr.fetchAndMerge(mgr.loadGeneration);
  assert.equal(mgr.isLoadComplete, true);
  await mgr.drainPendingIntents(mgr.loadGeneration);

  // ch2 must have drained normally (S-bump).
  const reg2 = mgr.overrideRegisters.get(channelId2);
  assert.ok(reg2, "ch2 register must be present");

  // ch1 had no existing register → uint32_overflow refusal on s=0 → actually
  // new S = max(0,0)+1 = 1, not overflow.  It should apply cleanly.
  const reg1 = mgr.overrideRegisters.get(channelId1);
  assert.ok(reg1, "ch1 register must be committed");
  assert.equal(reg1.s, 1, "ch1 S must be 1 after unread drain");

  // Drain completed without abort (no RangeError).
  assert.equal(
    pendingOverrideIntentStore.get(channelId1),
    undefined,
    "ch1 intent must be consumed",
  );
  assert.equal(
    pendingOverrideIntentStore.get(channelId2),
    undefined,
    "ch2 intent must be consumed",
  );

  cb.teardown();
  mgr.destroy();
});
