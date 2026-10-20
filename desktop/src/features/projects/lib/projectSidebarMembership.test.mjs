import assert from "node:assert/strict";
import test, { beforeEach } from "node:test";

import {
  __resetProjectSidebarMembershipForTests,
  addProjectToSidebar,
  mergeProjectSidebarMembershipStores,
  parseProjectSidebarMembershipPayload,
  PROJECT_SIDEBAR_MEMBERSHIP_EVENT,
  readProjectSidebarMembership,
  removeProjectFromSidebar,
  selectedProjectAddressesFromStore,
} from "./projectSidebarMembership.ts";

function store(projects = {}) {
  return { version: 1, projects };
}

const RELAY = "wss://relay.example.com";
const PUBKEY = "a".repeat(64);

const backingStore = new Map();
let failWrites = false;
let failReads = false;
globalThis.localStorage = {
  getItem: (key) => {
    if (failReads) throw new Error("storage read unavailable");
    return backingStore.get(key) ?? null;
  },
  setItem: (key, value) => {
    if (failWrites) throw new Error("storage write unavailable");
    backingStore.set(key, String(value));
  },
  removeItem: (key) => backingStore.delete(key),
};

/** Captures the membership dispatched with each mutation. */
function captureDispatches() {
  const dispatched = [];
  const previous = globalThis.dispatchEvent;
  globalThis.dispatchEvent = (event) => {
    if (event.type === PROJECT_SIDEBAR_MEMBERSHIP_EVENT) {
      dispatched.push(event.detail.addresses);
    }
    return true;
  };
  return {
    dispatched,
    stop: () => {
      globalThis.dispatchEvent = previous;
    },
  };
}

beforeEach(() => {
  backingStore.clear();
  failWrites = false;
  failReads = false;
  __resetProjectSidebarMembershipForTests();
});

test("migrates the legacy selected-address array", () => {
  const parsed = parseProjectSidebarMembershipPayload([
    "30621:alice:buzz",
    "30621:alice:buzz",
    "",
    42,
  ]);

  assert.deepEqual(parsed, {
    version: 1,
    projects: {
      "30621:alice:buzz": { selected: true, updatedAt: 0 },
    },
  });
});

test("merge preserves independent selections from two clients", () => {
  const merged = mergeProjectSidebarMembershipStores(
    store({
      "30621:alice:one": { selected: true, updatedAt: 100 },
    }),
    store({
      "30621:alice:two": { selected: true, updatedAt: 200 },
    }),
  );

  assert.deepEqual(selectedProjectAddressesFromStore(merged).sort(), [
    "30621:alice:one",
    "30621:alice:two",
  ]);
});

test("newer removal wins over an older selection", () => {
  const merged = mergeProjectSidebarMembershipStores(
    store({
      "30621:alice:buzz": { selected: true, updatedAt: 100 },
    }),
    store({
      "30621:alice:buzz": { selected: false, updatedAt: 200 },
    }),
  );

  assert.deepEqual(selectedProjectAddressesFromStore(merged), []);
});

test("equal-timestamp conflicts converge with removal winning", () => {
  const selected = store({
    "30621:alice:buzz": { selected: true, updatedAt: 100 },
  });
  const removed = store({
    "30621:alice:buzz": { selected: false, updatedAt: 100 },
  });

  assert.deepEqual(
    mergeProjectSidebarMembershipStores(selected, removed),
    mergeProjectSidebarMembershipStores(removed, selected),
  );
  assert.deepEqual(
    selectedProjectAddressesFromStore(
      mergeProjectSidebarMembershipStores(selected, removed),
    ),
    [],
  );
});

test("drops malformed entries and rejects unknown versions", () => {
  assert.deepEqual(
    parseProjectSidebarMembershipPayload({
      version: 1,
      projects: { project: { selected: "yes", updatedAt: 1 } },
    }),
    store(),
  );
  assert.equal(
    parseProjectSidebarMembershipPayload({ version: 2, projects: {} }),
    null,
  );
});

test("membership round-trips through storage", () => {
  addProjectToSidebar("30617:owner:alpha", RELAY, PUBKEY);
  addProjectToSidebar("30617:owner:beta", RELAY, PUBKEY);
  removeProjectFromSidebar("30617:owner:alpha", RELAY, PUBKEY);
  assert.deepEqual(readProjectSidebarMembership(RELAY, PUBKEY), [
    "30617:owner:beta",
  ]);
  // The persisted mirror matches the authoritative state.
  __resetProjectSidebarMembershipForTests();
  assert.deepEqual(readProjectSidebarMembership(RELAY, PUBKEY), [
    "30617:owner:beta",
  ]);
});

test("sequential mutations accumulate while every storage write fails", () => {
  failWrites = true;
  const { dispatched, stop } = captureDispatches();
  try {
    addProjectToSidebar("30617:owner:alpha", RELAY, PUBKEY);
    addProjectToSidebar("30617:owner:beta", RELAY, PUBKEY);
    removeProjectFromSidebar("30617:owner:alpha", RELAY, PUBKEY);
  } finally {
    stop();
  }
  // Each dispatch carries the full accumulated membership — not just the
  // latest change replayed over an empty store.
  assert.deepEqual(dispatched, [
    ["30617:owner:alpha"],
    ["30617:owner:alpha", "30617:owner:beta"],
    ["30617:owner:beta"],
  ]);
  assert.deepEqual(readProjectSidebarMembership(RELAY, PUBKEY), [
    "30617:owner:beta",
  ]);
});

test("mutations survive when both reads and writes fail", () => {
  failReads = true;
  failWrites = true;
  addProjectToSidebar("30617:owner:alpha", RELAY, PUBKEY);
  addProjectToSidebar("30617:owner:beta", RELAY, PUBKEY);
  assert.deepEqual(readProjectSidebarMembership(RELAY, PUBKEY), [
    "30617:owner:alpha",
    "30617:owner:beta",
  ]);
});

test("recovered persistence writes the accumulated membership back", () => {
  failWrites = true;
  addProjectToSidebar("30617:owner:alpha", RELAY, PUBKEY);
  failWrites = false;
  addProjectToSidebar("30617:owner:beta", RELAY, PUBKEY);
  __resetProjectSidebarMembershipForTests();
  // The write that succeeded persisted both entries, including the one whose
  // own write had failed.
  assert.deepEqual(readProjectSidebarMembership(RELAY, PUBKEY), [
    "30617:owner:alpha",
    "30617:owner:beta",
  ]);
});

test("scopes are independent", () => {
  failWrites = true;
  addProjectToSidebar("30617:owner:alpha", RELAY, PUBKEY);
  assert.deepEqual(readProjectSidebarMembership(RELAY, "b".repeat(64)), []);
  assert.deepEqual(
    readProjectSidebarMembership("wss://other.example.com", PUBKEY),
    [],
  );
});
