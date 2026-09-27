import assert from "node:assert/strict";
import test from "node:test";

import {
  bestieSessionStorageKey,
  clearBestieSessionBoundary,
  parseBestieSessionBoundary,
  readBestieSessionBoundary,
  writeBestieSessionBoundary,
} from "./bestieSessionStorage.ts";

const SCOPE = {
  agentPubkey: "A".repeat(64),
  ownerPubkey: "B".repeat(64),
  relayUrl: "wss://Example.COM/relay",
};

test("storage key is scoped by relay, owner, and agent", () => {
  const key = bestieSessionStorageKey(SCOPE);
  assert.match(key, /^buzz-bestie-session\.v1:/);
  assert.match(key, /a{64}/);
  assert.match(key, /b{64}/);
});

test("parseBestieSessionBoundary accepts valid payloads only", () => {
  assert.deepEqual(
    parseBestieSessionBoundary({
      baselineMessageIds: ["m1", 2, "m2"],
      firstMessageCreatedAt: 100,
      sessionRootId: "root-1",
    }),
    {
      baselineMessageIds: ["m1", "m2"],
      firstMessageCreatedAt: 100,
      sessionRootId: "root-1",
    },
  );
  assert.equal(parseBestieSessionBoundary(null), null);
  assert.equal(
    parseBestieSessionBoundary({
      baselineMessageIds: [],
      firstMessageCreatedAt: -1,
      sessionRootId: "root",
    }),
    null,
  );
});

test("read/write/clear round-trip through localStorage", () => {
  const memory = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => memory.get(key) ?? null,
      removeItem: (key) => {
        memory.delete(key);
      },
      setItem: (key, value) => {
        memory.set(key, String(value));
      },
    },
  };

  const boundary = {
    baselineMessageIds: ["old-1"],
    firstMessageCreatedAt: 42,
    sessionRootId: "root-1",
  };
  writeBestieSessionBoundary(SCOPE, boundary);
  assert.deepEqual(readBestieSessionBoundary(SCOPE), boundary);
  clearBestieSessionBoundary(SCOPE);
  assert.equal(readBestieSessionBoundary(SCOPE), null);
});

test("chevron dismiss keeps boundary; Close Thread clears it", () => {
  const memory = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => memory.get(key) ?? null,
      removeItem: (key) => {
        memory.delete(key);
      },
      setItem: (key, value) => {
        memory.set(key, String(value));
      },
    },
  };

  const boundary = {
    baselineMessageIds: ["old-1"],
    firstMessageCreatedAt: 42,
    sessionRootId: "root-1",
  };
  writeBestieSessionBoundary(SCOPE, boundary);
  // Simulate chevron dismiss: do not clear; reopen reads the same session.
  assert.deepEqual(readBestieSessionBoundary(SCOPE), boundary);
  // Simulate Close Thread: clear so next open starts blank.
  clearBestieSessionBoundary(SCOPE);
  assert.equal(readBestieSessionBoundary(SCOPE), null);
});
