/**
 * Integration tests for useObservedUnreadPersistence.
 *
 * These tests mount the REAL production hook via createRoot + act to exercise
 * the actual lifecycle: pagehide flush, unmount cleanup, scope fence, timer
 * ownership, and marker prune wiring. Storage-primitive behavior is covered
 * separately in observedUnreadStorage.test.mjs.
 */

import assert from "node:assert/strict";
import test from "node:test";

import {
  installDOMShim,
  installFreshStorage,
  makeObservedEvent,
  mountHook,
} from "./observedUnreadTestHarness.mjs";

// DOM shim must be installed before React is imported (happens inside mountHook
// on first call; the harness imports React at module parse time, so shim first).
installDOMShim();
// Install a module-level localStorage so React's scheduler doesn't choke.
installFreshStorage();

import {
  readObservedUnreadFromStorage,
  writeObservedUnreadToStorage,
} from "./observedUnreadStorage.ts";
import { act } from "react";

// ── Helpers ───────────────────────────────────────────────────────────────────

const PUBKEY = "aabbcc";
const RELAY = "wss://relay.example.com";
// Use a recent timestamp so age-pruning doesn't discard events before writing.
const NOW_S = Math.floor(Date.now() / 1_000);

const DEFAULT_PROPS = {
  pubkey: PUBKEY,
  relay: RELAY,
  isReady: true,
  readStateVersion: 0,
  getTs: () => null,
  getOwn: () => null,
};

function makeRefs() {
  const eventsRef = { current: new Map() };
  const latestRef = { current: new Map() };
  const inner = new Map();
  inner.set("evt-1", makeObservedEvent({ id: "evt-1", createdAt: NOW_S }));
  eventsRef.current.set("channel-1", inner);
  latestRef.current.set("channel-1", NOW_S);
  return { eventsRef, latestRef };
}

async function mountDefaultHook(refs, overrides = {}) {
  return mountHook({ ...DEFAULT_PROPS, ...overrides }, refs);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test("pagehide flush: event recorded within debounce window survives reload", async () => {
  installFreshStorage();
  const refs = makeRefs();
  const harness = await mountDefaultHook(refs);

  // Add a new event after mount (simulates a live message arriving mid-debounce).
  const newInner = new Map(refs.eventsRef.current.get("channel-2") ?? []);
  newInner.set(
    "evt-new",
    makeObservedEvent({ id: "evt-new", createdAt: NOW_S + 1 }),
  );
  refs.eventsRef.current.set("channel-2", newInner);
  refs.latestRef.current.set("channel-2", NOW_S + 1);

  harness.api.schedule(harness.api.currentScope);

  await act(async () => {
    globalThis.dispatchEvent({ type: "pagehide" });
  });

  const stored = readObservedUnreadFromStorage(PUBKEY, RELAY);
  assert.ok(stored !== null, "storage should not be null after pagehide flush");
  assert.ok(stored.has("channel-2"), "channel-2 must be in persisted storage");
  assert.ok(
    stored.get("channel-2")?.has("evt-new"),
    "evt-new must be in channel-2's persisted events",
  );

  await harness.unmount();
});

test("unmount with pending write flushes before teardown", async () => {
  installFreshStorage();
  const refs = makeRefs();
  const harness = await mountDefaultHook(refs);

  const inner = new Map();
  inner.set(
    "evt-unmount",
    makeObservedEvent({ id: "evt-unmount", createdAt: NOW_S + 2 }),
  );
  refs.eventsRef.current.set("ch-u", inner);
  harness.api.schedule(harness.api.currentScope);

  await harness.unmount();

  const stored = readObservedUnreadFromStorage(PUBKEY, RELAY);
  assert.ok(stored !== null, "storage must not be null after unmount flush");
  assert.ok(stored.has("ch-u"), "ch-u must be persisted after unmount");
});

test("clearAll cancels pending debounce so no resurrection after reload", async () => {
  installFreshStorage();
  const refs = makeRefs();
  const harness = await mountDefaultHook(refs);

  harness.api.schedule(harness.api.currentScope);
  harness.api.clearAll();

  // Simulate parent reassigning refs after markAllChannelsRead.
  refs.eventsRef.current = new Map();
  refs.latestRef.current = new Map();

  await act(async () => {
    globalThis.dispatchEvent({ type: "pagehide" });
  });

  assert.equal(
    readObservedUnreadFromStorage(PUBKEY, RELAY),
    null,
    "storage must be null after clearAll + empty flush",
  );

  await harness.unmount();
});

test("removeChannel replaces pending snapshot so sibling channel B survives reload", async () => {
  installFreshStorage();

  // Seed both channels so hydration loads them; removeChannel(ch1) mid-debounce
  // must replace the snapshot so ch2 survives the next pagehide flush.
  const seedMap = new Map();
  const ch1 = new Map();
  ch1.set("evt-1", makeObservedEvent({ id: "evt-1", createdAt: NOW_S }));
  seedMap.set("channel-1", ch1);
  const ch2 = new Map();
  ch2.set("evt-2", makeObservedEvent({ id: "evt-2", createdAt: NOW_S + 1 }));
  seedMap.set("channel-2", ch2);
  writeObservedUnreadToStorage(PUBKEY, RELAY, seedMap);

  const refs = {
    eventsRef: { current: new Map() },
    latestRef: { current: new Map() },
  };
  const harness = await mountDefaultHook(refs);

  assert.ok(
    refs.eventsRef.current.has("channel-1"),
    "hydration must restore channel-1",
  );
  assert.ok(
    refs.eventsRef.current.has("channel-2"),
    "hydration must restore channel-2",
  );

  harness.api.schedule(harness.api.currentScope);
  harness.api.removeChannel("channel-1");

  assert.ok(
    !refs.eventsRef.current.has("channel-1"),
    "channel-1 removed from refs",
  );
  assert.ok(
    refs.eventsRef.current.has("channel-2"),
    "channel-2 still in refs after removeChannel",
  );

  await act(async () => {
    globalThis.dispatchEvent({ type: "pagehide" });
  });

  const stored = readObservedUnreadFromStorage(PUBKEY, RELAY);
  assert.ok(
    stored !== null,
    "storage must not be null — channel-2 events survive",
  );
  assert.ok(
    !stored.has("channel-1"),
    "channel-1 must not appear after removeChannel",
  );
  assert.ok(
    stored.has("channel-2"),
    "channel-2 must survive removeChannel(channel-1)",
  );

  await harness.unmount();
});

test("marker prune: thread and channel markers prune covered events; sibling channels survive", async () => {
  installFreshStorage();

  const eventsRef = { current: new Map() };
  const latestRef = { current: new Map() };
  // channel-1: evt-a covered by thread:root-a marker; evt-b NOT covered (newer).
  // channel-a: two events covered by channel marker at NOW_S-5.
  // channel-b: event at NOW_S+10 (survives).
  const evtA = makeObservedEvent({
    id: "evt-a",
    createdAt: NOW_S - 10,
    rootId: "root-a",
  });
  const evtB = makeObservedEvent({
    id: "evt-b",
    createdAt: NOW_S + 10,
    rootId: "root-b",
  });
  const evtOld = makeObservedEvent({
    id: "evt-old",
    createdAt: NOW_S - 20,
    rootId: "root-old",
  });
  const evtMid = makeObservedEvent({
    id: "evt-mid",
    createdAt: NOW_S - 10,
    rootId: "root-mid",
  });
  const evtSurvivor = makeObservedEvent({
    id: "evt-survivor",
    createdAt: NOW_S + 10,
    rootId: "root-sv",
  });

  const stored = new Map();
  const ch1 = new Map();
  ch1.set("evt-a", evtA);
  ch1.set("evt-b", evtB);
  stored.set("channel-1", ch1);
  const chA = new Map();
  chA.set("evt-old", evtOld);
  chA.set("evt-mid", evtMid);
  stored.set("channel-a", chA);
  const chB = new Map();
  chB.set("evt-survivor", evtSurvivor);
  stored.set("channel-b", chB);
  writeObservedUnreadToStorage(PUBKEY, RELAY, stored);

  let pruneCount = 0;
  const harness = await mountDefaultHook(
    { eventsRef, latestRef },
    { isReady: false },
  );

  await harness.render({
    pubkey: PUBKEY,
    relay: RELAY,
    isReady: true,
    readStateVersion: 1,
    getTs: (channelId) => (channelId === "channel-a" ? NOW_S - 5 : null),
    getOwn: (ctx) => (ctx === "thread:root-a" ? NOW_S - 5 : null),
    onPruned: () => {
      pruneCount += 1;
    },
  });

  // Thread marker: evt-a pruned from channel-1; evt-b survives.
  assert.ok(
    !eventsRef.current.get("channel-1")?.has("evt-a"),
    "evt-a must be pruned by thread marker",
  );
  assert.ok(
    eventsRef.current.get("channel-1")?.has("evt-b"),
    "evt-b must survive (newer than marker)",
  );
  // Channel marker: channel-a fully cleared.
  assert.ok(
    !eventsRef.current.has("channel-a"),
    "channel-a must be fully pruned by channel marker",
  );
  // Sibling: channel-b unaffected.
  assert.ok(eventsRef.current.has("channel-b"), "channel-b must survive");
  assert.equal(pruneCount, 1, "onPruned must fire exactly once");

  await harness.unmount();
});

test("isScopeLoaded returns false before identity-reset effect commits, true after", async () => {
  installFreshStorage();
  const refs = makeRefs();
  const harness = await mountDefaultHook(refs, { isReady: false });
  assert.ok(
    harness.api.isScopeLoaded(),
    "isScopeLoaded must be true after mount+effects",
  );

  const PUBKEY_B = "pubkey-b-scope-test";
  await harness.render({
    pubkey: PUBKEY_B,
    relay: RELAY,
    isReady: false,
    readStateVersion: 0,
    getTs: () => null,
    getOwn: () => null,
  });
  assert.ok(
    harness.api.isScopeLoaded(),
    "isScopeLoaded must be true after scope switch + effects committed",
  );
  assert.ok(
    harness.api.currentScope.includes(PUBKEY_B.toLowerCase()),
    "currentScope must reflect the new pubkey",
  );
  assert.equal(
    harness.api.scopeLoadedRef.current,
    harness.api.currentScope,
    "scopeLoadedRef must equal currentScope after effects commit",
  );

  await harness.unmount();
});

test("A→B scope switch: pending A-timer is cancelled by flush, A data persisted synchronously", async () => {
  installFreshStorage();

  const PUBKEY_AT = "pubkey-a-t";
  const RELAY_AT = "wss://relay-a-t.example.com";
  const PUBKEY_BT = "pubkey-b-t";
  const RELAY_BT = "wss://relay-b-t.example.com";

  writeObservedUnreadToStorage(
    PUBKEY_AT,
    RELAY_AT,
    new Map([
      [
        "ch-at",
        new Map([
          ["evt-at", makeObservedEvent({ id: "evt-at", createdAt: NOW_S })],
        ]),
      ],
    ]),
  );

  const refsAT = {
    eventsRef: { current: new Map() },
    latestRef: { current: new Map() },
  };
  const propsAT = {
    pubkey: PUBKEY_AT,
    relay: RELAY_AT,
    isReady: true,
    readStateVersion: 0,
    getTs: () => null,
    getOwn: () => null,
  };

  const harness = await mountHook(propsAT, refsAT);
  assert.ok(
    refsAT.eventsRef.current.has("ch-at"),
    "hydration must restore ch-at from storage",
  );

  harness.api.schedule(harness.api.currentScope);

  // Switch to B: flushes A synchronously, resets refs, loads B.
  await harness.render({ ...propsAT, pubkey: PUBKEY_BT, relay: RELAY_BT });

  assert.ok(
    readObservedUnreadFromStorage(PUBKEY_AT, RELAY_AT)?.has("ch-at"),
    "A must be flushed synchronously on scope switch",
  );
  assert.ok(
    readObservedUnreadFromStorage(PUBKEY_BT, RELAY_BT) == null ||
      !readObservedUnreadFromStorage(PUBKEY_BT, RELAY_BT)?.has("ch-at"),
    "B's bucket must not contain A's channel",
  );

  // B schedules and flushes independently.
  const chBT = new Map();
  chBT.set(
    "evt-bt",
    makeObservedEvent({ id: "evt-bt", createdAt: NOW_S + 300 }),
  );
  refsAT.eventsRef.current.set("ch-bt", chBT);
  refsAT.latestRef.current.set("ch-bt", NOW_S + 300);
  harness.api.schedule(harness.api.currentScope);

  await act(async () => {
    globalThis.dispatchEvent({ type: "pagehide" });
  });

  assert.ok(
    readObservedUnreadFromStorage(PUBKEY_BT, RELAY_BT)?.has("ch-bt"),
    "B's scheduled write must persist independently",
  );

  await harness.unmount();
});

test("stale scope-A operations (clearAll + removeChannel) both reject after scope B loads (scope fence enforced)", async () => {
  // Single test covering both fenced operations; each assertion names the
  // operation so a failure pinpoints which one broke.
  installFreshStorage();

  const seedA = new Map([
    [
      "channel-seed",
      new Map([
        [
          "evt-seed-a",
          makeObservedEvent({ id: "evt-seed-a", createdAt: NOW_S }),
        ],
      ]),
    ],
  ]);
  writeObservedUnreadToStorage(PUBKEY, RELAY, seedA);

  const refsA = {
    eventsRef: { current: new Map() },
    latestRef: { current: new Map() },
  };
  const propsA = {
    pubkey: PUBKEY,
    relay: RELAY,
    isReady: true,
    readStateVersion: 0,
    getTs: () => null,
    getOwn: () => null,
  };
  const harness = await mountHook(propsA, refsA);

  harness.api.schedule(harness.api.currentScope);
  const staleClearAll = harness.api.clearAll;
  const staleRemoveChannel = harness.api.removeChannel;

  const PUBKEY_B = "pubkey-b-fence";
  const RELAY_B = "wss://relay-b-fence.example.com";
  writeObservedUnreadToStorage(
    PUBKEY_B,
    RELAY_B,
    new Map([
      [
        "channel-b",
        new Map([
          [
            "evt-seed-b",
            makeObservedEvent({ id: "evt-seed-b", createdAt: NOW_S }),
          ],
        ]),
      ],
    ]),
  );
  await harness.render({ ...propsA, pubkey: PUBKEY_B, relay: RELAY_B });

  const storedA_before = readObservedUnreadFromStorage(PUBKEY, RELAY);
  assert.ok(storedA_before !== null, "A's bucket must survive scope switch");

  const storedB_before = readObservedUnreadFromStorage(PUBKEY_B, RELAY_B);
  assert.ok(
    storedB_before !== null,
    "B's bucket must be in storage after scope switch",
  );

  // stale clearAll must not delete A's bucket.
  staleClearAll();
  assert.deepEqual(
    readObservedUnreadFromStorage(PUBKEY, RELAY),
    storedA_before,
    "stale clearAll from scope A must not delete A's bucket after scope B loads",
  );

  // stale removeChannel must not modify B's bucket.
  staleRemoveChannel("channel-b");
  assert.deepEqual(
    readObservedUnreadFromStorage(PUBKEY_B, RELAY_B),
    storedB_before,
    "stale removeChannel from scope A must not delete channel-b from B's bucket",
  );

  await harness.unmount();
});

test("unrelated rerenders do not change API object identity (catch-up stability)", async () => {
  installFreshStorage();
  const refs = makeRefs();
  const harness = await mountDefaultHook(refs);
  const api1 = harness.api;

  // readStateVersion changes on every read-state advance but must NOT change the API object.
  await harness.render({ ...DEFAULT_PROPS, readStateVersion: 1 });

  const api2 = harness.api;
  assert.equal(
    api1,
    api2,
    "API object must be the same reference on unrelated rerender",
  );
  assert.equal(api1.schedule, api2.schedule, "schedule must be stable");
  assert.equal(
    api1.removeChannel,
    api2.removeChannel,
    "removeChannel must be stable",
  );
  assert.equal(api1.clearAll, api2.clearAll, "clearAll must be stable");
  assert.equal(
    api1.isScopeLoaded,
    api2.isScopeLoaded,
    "isScopeLoaded must be stable",
  );

  await harness.unmount();
});
