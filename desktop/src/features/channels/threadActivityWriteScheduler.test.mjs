/**
 * Unit tests for the coalesced thread-activity write scheduler.
 *
 * These exercise scheduleThreadActivityWrite / flushThreadActivityWrite /
 * removeLegacyThreadActivityKey directly against a ref bag, using node:test
 * fake timers to drive the 1s debounce deterministically. Hook lifecycle
 * (pagehide/visibility flush, scope-switch hydration) is covered separately in
 * useThreadActivityPersistence.test.mjs.
 */

import assert from "node:assert/strict";
import { afterEach, mock, test } from "node:test";

import {
  activityScopeKey,
  activityStorageKey,
  flushThreadActivityWrite,
  readActivityFromStorage,
  removeLegacyThreadActivityKey,
  scheduleThreadActivityWrite,
} from "./threadActivityStorage.ts";

const originalWindow = globalThis.window;

afterEach(() => {
  mock.timers.reset();
  if (originalWindow === undefined) delete globalThis.window;
  else globalThis.window = originalWindow;
});

// Install an isolated in-memory localStorage with a setItem counter so a burst
// can be asserted to collapse to exactly one write.
function installStore() {
  const store = new Map();
  let setItemCalls = 0;
  globalThis.window = {
    localStorage: {
      getItem: (key) => store.get(key) ?? null,
      setItem: (key, value) => {
        setItemCalls += 1;
        store.set(key, value);
      },
      removeItem: (key) => store.delete(key),
    },
  };
  return { store, setItemCalls: () => setItemCalls };
}

function makeRefs(scope, items = []) {
  return {
    itemsRef: { current: items },
    scopeLoadedRef: { current: scope },
    timerRef: { current: null },
  };
}

const RELAY = "wss://relay.example.com";

// ── scheduleThreadActivityWrite ──────────────────────────────────────────────

test("scheduleThreadActivityWrite coalesces a burst of schedules into one setItem", () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  const { setItemCalls } = installStore();

  const scope = activityScopeKey("pk1", RELAY);
  const refs = makeRefs(scope, [{ id: "r1" }]);

  for (let i = 0; i < 5; i += 1) scheduleThreadActivityWrite(scope, refs);
  assert.equal(setItemCalls(), 0, "no write before the debounce elapses");

  mock.timers.tick(1_000);
  assert.equal(
    setItemCalls(),
    1,
    "a burst of 5 schedules fires exactly one write",
  );
});

test("scheduleThreadActivityWrite persists the live buffer at fire time, not at schedule time", () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  installStore();

  const scope = activityScopeKey("pk1", RELAY);
  const refs = makeRefs(scope, [{ id: "early" }]);

  scheduleThreadActivityWrite(scope, refs);
  // A second reply lands before the timer fires — the buffer grows in place.
  refs.itemsRef.current = [{ id: "early" }, { id: "late" }];

  mock.timers.tick(1_000);
  assert.deepEqual(
    readActivityFromStorage("pk1", RELAY).map((item) => item.id),
    ["early", "late"],
    "the persisted blob reflects the buffer's final state",
  );
});

test("scheduleThreadActivityWrite ignores a scope that does not match the loaded scope", () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  const { setItemCalls } = installStore();

  const refs = makeRefs(activityScopeKey("pkA", "wss://relay-a.example.com"), [
    { id: "a1" },
  ]);
  scheduleThreadActivityWrite(
    activityScopeKey("pkB", "wss://relay-b.example.com"),
    refs,
  );

  assert.equal(
    refs.timerRef.current,
    null,
    "no timer armed for a mismatched scope",
  );
  mock.timers.tick(1_000);
  assert.equal(setItemCalls(), 0, "a mismatched scope never writes");
});

test("scheduleThreadActivityWrite timer aborts when the loaded scope changed before it fired", () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  const { setItemCalls } = installStore();

  const pkA = "pkA";
  const relayA = "wss://relay-a.example.com";
  const scopeA = activityScopeKey(pkA, relayA);
  const refs = makeRefs(scopeA, [{ id: "a1" }]);

  scheduleThreadActivityWrite(scopeA, refs);
  // Scope switches out from under the pending timer without cancelling it.
  refs.scopeLoadedRef.current = activityScopeKey(
    "pkB",
    "wss://relay-b.example.com",
  );

  mock.timers.tick(1_000);
  assert.equal(setItemCalls(), 0, "a stale-scope timer must not write");
  assert.equal(
    readActivityFromStorage(pkA, relayA).length,
    0,
    "A's key must stay empty when the timer aborts",
  );
});

// ── flushThreadActivityWrite ─────────────────────────────────────────────────

test("flushThreadActivityWrite persists synchronously and cancels the pending timer", () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  const { setItemCalls } = installStore();

  const scope = activityScopeKey("pk1", RELAY);
  const refs = makeRefs(scope, [{ id: "f1" }]);

  scheduleThreadActivityWrite(scope, refs);
  flushThreadActivityWrite(refs);

  assert.equal(setItemCalls(), 1, "flush writes immediately");
  assert.equal(refs.timerRef.current, null, "flush cancels the timer");
  assert.deepEqual(
    readActivityFromStorage("pk1", RELAY).map((item) => item.id),
    ["f1"],
  );

  mock.timers.tick(1_000);
  assert.equal(
    setItemCalls(),
    1,
    "the cancelled timer never fires a second write",
  );
});

test("flushThreadActivityWrite is a no-op when no write is pending", () => {
  const { setItemCalls } = installStore();

  const refs = makeRefs(activityScopeKey("pk1", RELAY), [{ id: "x" }]);
  flushThreadActivityWrite(refs);

  assert.equal(setItemCalls(), 0, "no pending timer means no write");
});

// ── removeLegacyThreadActivityKey ────────────────────────────────────────────

test("removeLegacyThreadActivityKey removes the orphaned pubkey-only key and preserves the scoped key", () => {
  const { store } = installStore();

  const pubkey = "pk1";
  const legacyKey = `buzz-thread-activity.v1:${pubkey}`;
  const scopedKey = activityStorageKey(pubkey, RELAY);
  store.set(legacyKey, JSON.stringify([{ id: "legacy" }]));
  store.set(scopedKey, JSON.stringify([{ id: "scoped" }]));

  removeLegacyThreadActivityKey(pubkey);
  assert.equal(store.has(legacyKey), false, "legacy pubkey-only key removed");
  assert.equal(store.has(scopedKey), true, "relay-scoped key preserved");

  // Idempotent: a second call on the absent key is a no-op that never throws.
  removeLegacyThreadActivityKey(pubkey);
  assert.equal(store.has(legacyKey), false);
});
