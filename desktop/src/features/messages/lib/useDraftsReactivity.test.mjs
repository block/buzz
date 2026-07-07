/**
 * Unit tests for useDrafts store reactivity.
 *
 * Tests verify the actual subscriber-notification contract:
 *   - A write path (saveDraftEntry / clearDraftEntry / persistDraftEntry /
 *     markDraftSentEntry) fires each registered subscriber exactly once.
 *   - The snapshot version increments on each write.
 *   - Unsubscribed callbacks are NOT called.
 *   - clearDraftEntry on a nonexistent key is a no-op (no notification).
 *
 * Uses the exported `subscribeToStore` + `getStoreSnapshot` primitives so the
 * contract is tested at the source — no React renderer needed.
 */

import assert from "node:assert/strict";
import test from "node:test";

// ── Browser-global shim ───────────────────────────────────────────────────────

function makeLocalStorage() {
  const store = new Map();
  return {
    get length() {
      return store.size;
    },
    key: (i) => [...store.keys()][i] ?? null,
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, value),
    removeItem: (key) => store.delete(key),
    clear: () => store.clear(),
  };
}

function installFreshLocalStorage() {
  const ls = makeLocalStorage();
  if (typeof globalThis.window === "undefined") {
    globalThis.window = { localStorage: ls };
  } else {
    globalThis.window.localStorage = ls;
  }
  Object.defineProperty(globalThis, "localStorage", {
    get: () => globalThis.window.localStorage,
    configurable: true,
  });
  return ls;
}

installFreshLocalStorage();

import {
  clearAllDrafts,
  clearDraftEntry,
  getStoreSnapshot,
  initDraftStore,
  markDraftSentEntry,
  persistDraftEntry,
  saveDraftEntry,
  subscribeToStore,
} from "./useDrafts.ts";

// ── Helpers ───────────────────────────────────────────────────────────────────

function setup(pubkey = "pubkey-reactivity") {
  installFreshLocalStorage();
  clearAllDrafts();
  initDraftStore(pubkey);
}

function makeDraft(overrides = {}) {
  const now = new Date().toISOString();
  return {
    content: "hello",
    selectionStart: 5,
    selectionEnd: 5,
    channelId: "chan-1",
    createdAt: now,
    updatedAt: now,
    pendingImeta: [],
    spoileredAttachmentUrls: [],
    status: "active",
    ...overrides,
  };
}

/**
 * Subscribe, run `action`, unsubscribe, and return {callCount, versionBefore, versionAfter}.
 */
function observeWrite(action) {
  let callCount = 0;
  const versionBefore = getStoreSnapshot();
  const unsub = subscribeToStore(() => {
    callCount += 1;
  });
  action();
  unsub();
  const versionAfter = getStoreSnapshot();
  return { callCount, versionBefore, versionAfter };
}

// ── saveDraftEntry notifies subscriber and bumps version ──────────────────────

test("saveDraftEntry_notifies_subscriber_and_bumps_version", () => {
  setup();
  const { callCount, versionBefore, versionAfter } = observeWrite(() => {
    saveDraftEntry("chan-save", makeDraft());
  });
  assert.equal(callCount, 1, "subscriber must be called exactly once");
  assert.equal(
    versionAfter,
    versionBefore + 1,
    "version must increment by 1 on saveDraftEntry",
  );
});

// ── clearDraftEntry notifies when key exists ──────────────────────────────────

test("clearDraftEntry_notifies_subscriber_when_key_exists", () => {
  setup();
  saveDraftEntry("chan-del", makeDraft({ content: "to delete" }));
  const { callCount, versionBefore, versionAfter } = observeWrite(() => {
    clearDraftEntry("chan-del");
  });
  assert.equal(
    callCount,
    1,
    "subscriber must be called exactly once on delete",
  );
  assert.equal(
    versionAfter,
    versionBefore + 1,
    "version must increment on delete",
  );
});

// ── clearDraftEntry on nonexistent key is a no-op ─────────────────────────────

test("clearDraftEntry_is_noop_for_nonexistent_key", () => {
  setup();
  const { callCount, versionBefore, versionAfter } = observeWrite(() => {
    clearDraftEntry("key-that-does-not-exist");
  });
  assert.equal(
    callCount,
    0,
    "subscriber must NOT be called for nonexistent key",
  );
  assert.equal(
    versionAfter,
    versionBefore,
    "version must NOT change for nonexistent key",
  );
});

// ── persistDraftEntry notifies on save ───────────────────────────────────────

test("persistDraftEntry_non_empty_notifies_subscriber", () => {
  setup();
  const { callCount, versionBefore, versionAfter } = observeWrite(() => {
    persistDraftEntry("chan-p", "some content", "chan-p", [], []);
  });
  assert.equal(callCount, 1, "subscriber must be called on persist");
  assert.equal(
    versionAfter,
    versionBefore + 1,
    "version must increment on persist",
  );
});

// ── persistDraftEntry clears on empty content ─────────────────────────────────

test("persistDraftEntry_empty_content_notifies_subscriber", () => {
  setup();
  // First save something.
  persistDraftEntry("chan-p2", "will be cleared", "chan-p2", [], []);
  const { callCount, versionBefore, versionAfter } = observeWrite(() => {
    // Whitespace-only triggers a clear.
    persistDraftEntry("chan-p2", "   ", "chan-p2", [], []);
  });
  assert.equal(
    callCount,
    1,
    "subscriber must be called when content is cleared",
  );
  assert.equal(
    versionAfter,
    versionBefore + 1,
    "version must increment on clear",
  );
});

// ── markDraftSentEntry notifies subscriber ────────────────────────────────────

test("markDraftSentEntry_notifies_subscriber", () => {
  setup();
  persistDraftEntry("chan-sent", "content to send", "chan-sent", [], []);
  const { callCount, versionBefore, versionAfter } = observeWrite(() => {
    markDraftSentEntry("chan-sent", "content to send", "chan-sent", [], []);
  });
  assert.equal(callCount, 1, "subscriber must be called on markSent");
  assert.equal(
    versionAfter,
    versionBefore + 1,
    "version must increment on markSent",
  );
});

// ── Unsubscribed callback is NOT called ───────────────────────────────────────

test("unsubscribed_callback_is_not_called", () => {
  setup();
  let callCount = 0;
  const unsub = subscribeToStore(() => {
    callCount += 1;
  });
  // Immediately unsubscribe before the write.
  unsub();
  saveDraftEntry("chan-unsub", makeDraft());
  assert.equal(callCount, 0, "unsubscribed callback must NOT be called");
});

// ── Multiple writes each bump version independently ──────────────────────────

test("multiple_writes_each_bump_version_independently", () => {
  setup();
  const v0 = getStoreSnapshot();
  saveDraftEntry("chan-a", makeDraft());
  const v1 = getStoreSnapshot();
  saveDraftEntry("chan-b", makeDraft());
  const v2 = getStoreSnapshot();
  assert.equal(v1, v0 + 1, "first write must bump version");
  assert.equal(v2, v1 + 1, "second write must bump version again");
});
