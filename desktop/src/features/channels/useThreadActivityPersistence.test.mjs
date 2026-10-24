/**
 * Integration tests for useThreadActivityPersistence.
 *
 * These mount the REAL production hook via createRoot + act to exercise the
 * actual lifecycle: pagehide flush, visibilitychange→hidden flush, unmount
 * cleanup, scope-switch hydration (flush-before-reseed), legacy-key cleanup,
 * and the read-live-buffer-at-flush-time contract that distinguishes this
 * scheduler from a snapshot-at-schedule one. Debounce timing is covered by
 * fake timers in threadActivityWriteScheduler.test.mjs.
 */

import assert from "node:assert/strict";
import test from "node:test";

import {
  installDOMShim,
  installFreshStorage,
} from "./observedUnreadTestHarness.mjs";

installDOMShim();
installFreshStorage();

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import {
  activityStorageKey,
  readActivityFromStorage,
  writeActivityToStorage,
} from "./threadActivityStorage.ts";
import { useThreadActivityPersistence } from "./useThreadActivityPersistence.ts";

const RELAY = "wss://relay.example.com";

// Mount the hook with a caller-owned buffer ref (mirrors useUnreadChannels,
// which owns threadActivityRef and passes it in).
async function mountHook(itemsRef, props) {
  const apiRef = { current: null };

  function Harness({ pubkey, relay }) {
    apiRef.current = useThreadActivityPersistence(pubkey, relay, itemsRef);
    return null;
  }

  const root = createRoot(document.createElement("div"));
  const render = async (p) => {
    await act(async () => {
      root.render(React.createElement(Harness, p));
    });
  };
  await render(props);

  return {
    get api() {
      return apiRef.current;
    },
    render,
    unmount: async () => {
      await act(async () => root.unmount());
    },
  };
}

// ── flush contract: read the live buffer at flush time ───────────────────────

test("pagehide flush persists the live buffer's final state, not a schedule-time snapshot", async () => {
  installFreshStorage();
  const itemsRef = { current: [] };
  const harness = await mountHook(itemsRef, { pubkey: "pk1", relay: RELAY });

  // A reply lands, the writer buffers it in place and arms a coalesced write.
  itemsRef.current = [{ id: "early" }];
  harness.api.schedule(harness.api.currentScope);
  // A second reply lands within the debounce window — buffer grows in place.
  itemsRef.current = [{ id: "early" }, { id: "late" }];

  await act(async () => {
    globalThis.dispatchEvent({ type: "pagehide" });
  });

  assert.deepEqual(
    readActivityFromStorage("pk1", RELAY).map((item) => item.id),
    ["early", "late"],
    "flush must persist the buffer's final state including the late reply",
  );

  await harness.unmount();
});

test("visibilitychange to hidden flushes a pending write", async () => {
  installFreshStorage();
  const itemsRef = { current: [] };
  const harness = await mountHook(itemsRef, { pubkey: "pk1", relay: RELAY });

  itemsRef.current = [{ id: "hidden-flush" }];
  harness.api.schedule(harness.api.currentScope);

  await act(async () => {
    globalThis.document.visibilityState = "hidden";
    globalThis.document.dispatchEvent({ type: "visibilitychange" });
  });

  assert.deepEqual(
    readActivityFromStorage("pk1", RELAY).map((item) => item.id),
    ["hidden-flush"],
    "backgrounding the webview must persist the pending buffer",
  );

  await harness.unmount();
});

test("visibilitychange to visible does not write", async () => {
  installFreshStorage();
  const itemsRef = { current: [] };
  const harness = await mountHook(itemsRef, { pubkey: "pk1", relay: RELAY });

  itemsRef.current = [{ id: "still-buffered" }];
  harness.api.schedule(harness.api.currentScope);

  await act(async () => {
    globalThis.document.visibilityState = "visible";
    globalThis.document.dispatchEvent({ type: "visibilitychange" });
  });

  assert.deepEqual(
    readActivityFromStorage("pk1", RELAY),
    [],
    "a visible transition must not flush",
  );

  await harness.unmount();
});

test("unmount with a pending write flushes before teardown", async () => {
  installFreshStorage();
  const itemsRef = { current: [] };
  const harness = await mountHook(itemsRef, { pubkey: "pk1", relay: RELAY });

  itemsRef.current = [{ id: "unmount-flush" }];
  harness.api.schedule(harness.api.currentScope);

  await harness.unmount();

  assert.deepEqual(
    readActivityFromStorage("pk1", RELAY).map((item) => item.id),
    ["unmount-flush"],
    "unmount cleanup must flush the pending write",
  );
});

// ── scope switch: flush-before-reseed under the OLD key ──────────────────────

test("scope switch flushes A synchronously under A's key and does not leak A into B", async () => {
  installFreshStorage();

  const pkA = "pkA";
  const relayA = "wss://relay-a.example.com";
  const pkB = "pkB";
  const relayB = "wss://relay-b.example.com";

  const itemsRef = { current: [] };
  const harness = await mountHook(itemsRef, { pubkey: pkA, relay: relayA });

  // A accumulates a buffered reply with a pending coalesced write.
  itemsRef.current = [{ id: "a-only" }];
  harness.api.schedule(harness.api.currentScope);

  // Switch identity to B: the hydration effect must flush A first.
  await harness.render({ pubkey: pkB, relay: relayB });

  assert.deepEqual(
    readActivityFromStorage(pkA, relayA).map((item) => item.id),
    ["a-only"],
    "A's pending write must land under A's key on scope switch",
  );
  assert.deepEqual(
    readActivityFromStorage(pkB, relayB),
    [],
    "B's bucket must not contain A's rows",
  );
  assert.ok(
    harness.api.currentScope.includes(pkB),
    "currentScope must reflect B after the switch",
  );

  await harness.unmount();
});

test("scope switch hydrates B's buffer from B's persisted bucket", async () => {
  installFreshStorage();

  const pkA = "pkA";
  const relayA = "wss://relay-a.example.com";
  const pkB = "pkB";
  const relayB = "wss://relay-b.example.com";
  writeActivityToStorage(pkB, relayB, [{ id: "b-persisted" }]);

  const itemsRef = { current: [] };
  const harness = await mountHook(itemsRef, { pubkey: pkA, relay: relayA });

  itemsRef.current = [{ id: "a-only" }];
  await harness.render({ pubkey: pkB, relay: relayB });

  assert.deepEqual(
    itemsRef.current.map((item) => item.id),
    ["b-persisted"],
    "the buffer must be reseeded from B's bucket, dropping A's rows",
  );

  await harness.unmount();
});

// ── legacy key cleanup ───────────────────────────────────────────────────────

test("mounting removes the orphaned legacy pubkey-only key", async () => {
  const ls = installFreshStorage();
  const pubkey = "pk-legacy";
  const legacyKey = `buzz-thread-activity.v1:${pubkey}`;
  ls.setItem(legacyKey, JSON.stringify([{ id: "stale" }]));

  const itemsRef = { current: [] };
  const harness = await mountHook(itemsRef, { pubkey, relay: RELAY });

  assert.equal(
    ls.getItem(legacyKey),
    null,
    "hydration must drop the orphaned legacy key",
  );

  await harness.unmount();
});

// ── scope fence: never write before a valid scope is loaded ──────────────────

test("isScopeLoaded is false without an identity and true after hydration", async () => {
  installFreshStorage();
  const itemsRef = { current: [] };
  const harness = await mountHook(itemsRef, { pubkey: null, relay: RELAY });

  assert.equal(
    harness.api.isScopeLoaded(),
    false,
    "an absent pubkey must never pass the scope fence",
  );

  await harness.render({ pubkey: "pk1", relay: RELAY });
  assert.equal(
    harness.api.isScopeLoaded(),
    true,
    "a valid scope must pass once its hydration effect commits",
  );

  await harness.unmount();
});

test("schedule under an empty scope never writes", async () => {
  const ls = installFreshStorage();
  const itemsRef = { current: [] };
  const harness = await mountHook(itemsRef, { pubkey: null, relay: RELAY });

  itemsRef.current = [{ id: "orphan" }];
  harness.api.schedule(harness.api.currentScope);

  await act(async () => {
    globalThis.dispatchEvent({ type: "pagehide" });
  });

  assert.equal(
    ls.getItem(activityStorageKey("", RELAY)),
    null,
    "no write may land when identity is unknown",
  );

  await harness.unmount();
});
