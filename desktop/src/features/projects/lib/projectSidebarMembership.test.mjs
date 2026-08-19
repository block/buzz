import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import {
  __resetProjectSidebarMembershipForTests,
  addProjectToSidebar,
  PROJECT_SIDEBAR_MEMBERSHIP_EVENT,
  readProjectSidebarMembership,
  removeProjectFromSidebar,
} from "./projectSidebarMembership.ts";

const RELAY = "wss://relay.example.com";
const PUBKEY = "a".repeat(64);

const store = new Map();
let failWrites = false;
let failReads = false;
globalThis.localStorage = {
  getItem: (key) => {
    if (failReads) throw new Error("storage read unavailable");
    return store.get(key) ?? null;
  },
  setItem: (key, value) => {
    if (failWrites) throw new Error("storage write unavailable");
    store.set(key, String(value));
  },
  removeItem: (key) => store.delete(key),
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
  store.clear();
  failWrites = false;
  failReads = false;
  __resetProjectSidebarMembershipForTests();
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
