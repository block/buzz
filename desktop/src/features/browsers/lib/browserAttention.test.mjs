import assert from "node:assert/strict";
import test from "node:test";

import { installLocalStorage } from "../../playground/lib/testStorage.mjs";

import {
  browserAttentionStorageKey,
  countNewBrowserGroups,
  dropBrowserGroupsSeen,
  EMPTY_BROWSER_ATTENTION,
  loadBrowserAttentionState,
  markBrowserGroupsSeen,
  parseBrowserAttentionState,
  reconcileBrowserAttention,
  saveBrowserAttentionState,
  seedBrowserAttention,
} from "./browserAttention.ts";
import {
  configureBrowserAttentionScope,
  getBrowserAttentionState,
  markAllListedBrowserGroupsSeen,
  markBrowserGroupsAsSeen,
  reconcileBrowserAttentionWithRoster,
  resetBrowserAttentionStore,
  __setBrowserAttentionStateForTests,
} from "./browserAttentionStore.ts";

test("browserAttentionStorageKey scopes with pubkey+relay or falls back", () => {
  assert.equal(browserAttentionStorageKey(), "buzz-browser-attention.v1");
  assert.equal(browserAttentionStorageKey("", ""), "buzz-browser-attention.v1");
  const scoped = browserAttentionStorageKey("pk", "wss://Relay.Example/");
  assert.ok(scoped.startsWith("buzz-browser-attention.v1:pk:"));
  assert.ok(scoped.includes(encodeURIComponent("wss://relay.example")));
});

test("countNewBrowserGroups counts unseen ids only", () => {
  assert.equal(countNewBrowserGroups(["a", "b", "c"], ["a"]), 2);
  assert.equal(countNewBrowserGroups(["a"], new Set(["a", "b"])), 0);
  assert.equal(countNewBrowserGroups([], []), 0);
  assert.equal(countNewBrowserGroups(["", "x"], []), 1);
});

test("seedBrowserAttention marks all current as seen", () => {
  const seeded = seedBrowserAttention(["g1", "g2", "g1"]);
  assert.equal(seeded.seeded, true);
  assert.deepEqual(seeded.seenIds.sort(), ["g1", "g2"]);
  assert.equal(countNewBrowserGroups(["g1", "g2"], seeded.seenIds), 0);
  assert.equal(countNewBrowserGroups(["g1", "g2", "g3"], seeded.seenIds), 1);
});

test("markBrowserGroupsSeen and dropBrowserGroupsSeen", () => {
  let state = seedBrowserAttention(["a"]);
  state = markBrowserGroupsSeen(state, ["b", "c"]);
  assert.deepEqual([...state.seenIds].sort(), ["a", "b", "c"]);
  const same = markBrowserGroupsSeen(state, ["b"]);
  assert.equal(same, state);
  state = dropBrowserGroupsSeen(state, ["b", "missing"]);
  assert.deepEqual([...state.seenIds].sort(), ["a", "c"]);
});

test("reconcileBrowserAttention seeds once then drops disposed", () => {
  const first = reconcileBrowserAttention(EMPTY_BROWSER_ATTENTION, ["x", "y"]);
  assert.equal(first.seeded, true);
  assert.deepEqual([...first.seenIds].sort(), ["x", "y"]);
  // New group is not auto-seen.
  const withNew = reconcileBrowserAttention(first, ["x", "y", "z"]);
  assert.equal(withNew, first);
  assert.equal(countNewBrowserGroups(["x", "y", "z"], first.seenIds), 1);
  // Disposed group drops from seen.
  const afterDrop = reconcileBrowserAttention(first, ["x"]);
  assert.deepEqual(afterDrop.seenIds, ["x"]);
});

test("parseBrowserAttentionState tolerates bad input", () => {
  assert.deepEqual(parseBrowserAttentionState(null), EMPTY_BROWSER_ATTENTION);
  assert.deepEqual(
    parseBrowserAttentionState({ seeded: true, seenIds: [1, "a"] }),
    {
      version: 1,
      seeded: true,
      seenIds: ["a"],
    },
  );
});

test("load/save round-trip via safeStorage", () => {
  installLocalStorage();
  const key = browserAttentionStorageKey("pk", "wss://r/");
  const state = seedBrowserAttention(["one"]);
  assert.equal(saveBrowserAttentionState(key, state), true);
  assert.deepEqual(loadBrowserAttentionState(key), state);
});

test("store: seed then new browser increments; mark-all clears", () => {
  installLocalStorage();
  resetBrowserAttentionStore();
  configureBrowserAttentionScope("pk", "wss://relay.test/");
  reconcileBrowserAttentionWithRoster(["g1", "g2"]);
  assert.equal(getBrowserAttentionState().seeded, true);
  assert.equal(
    countNewBrowserGroups(
      ["g1", "g2", "g3"],
      getBrowserAttentionState().seenIds,
    ),
    1,
  );
  // Simulate roster growing without reconcile auto-seeing.
  __setBrowserAttentionStateForTests(getBrowserAttentionState());
  markBrowserGroupsAsSeen(["g3"]);
  assert.equal(
    countNewBrowserGroups(
      ["g1", "g2", "g3"],
      getBrowserAttentionState().seenIds,
    ),
    0,
  );
  __setBrowserAttentionStateForTests(seedBrowserAttention(["a"]));
  // New id while seeded:
  __setBrowserAttentionStateForTests({
    version: 1,
    seeded: true,
    seenIds: ["a"],
  });
  markAllListedBrowserGroupsSeen(["a", "b", "c"]);
  assert.equal(
    countNewBrowserGroups(["a", "b", "c"], getBrowserAttentionState().seenIds),
    0,
  );
});
