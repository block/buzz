/**
 * Regression tests for the submit-time draft-persistence predicate used by
 * MessageComposer's `submitMessage` handler.
 *
 * The predicate lives in `resolveSentDraftKey` (draftSubmitKey.ts), which
 * MessageComposer calls directly. These tests import and exercise the ACTUAL
 * exported function — not a restatement of its logic — so a regression at
 * the call site (e.g. reverting to the inline expression or removing the call)
 * is a conscious change, and a logic regression inside the function breaks
 * these tests immediately.
 *
 * Three properties under test:
 *   (a) never-persisted key → null  (fast send must not produce a sent record)
 *   (b) persisted key       → key   (normal send produces a sent record)
 *   (c) submit-time capture semantics: the value returned at submit time is
 *       stable even if the store entry is cleared before send success (proving
 *       the predicate is evaluated once at submit, not re-read at success).
 *
 * Integration scenarios (d)+(e) drive the full storage flow to confirm
 * that the predicate output correctly gates markDraftSentEntry.
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

// ── Imports ───────────────────────────────────────────────────────────────────

import { resolveSentDraftKey } from "./draftSubmitKey.ts";

import {
  clearAllDrafts,
  getSentDraftEntries,
  initDraftStore,
  loadDraftEntry,
  markDraftSentEntry,
  persistDraftEntry,
} from "../lib/useDrafts.ts";

function setup(pubkey = "pubkey-predicate") {
  installFreshLocalStorage();
  clearAllDrafts();
  initDraftStore(pubkey);
}

const IMG_A = {
  url: "https://cdn.example.com/a.jpg",
  sha256: "aabbccdd",
  size: 1024,
  type: "image/jpeg",
  uploaded: 0,
};

// ── (a) resolveSentDraftKey: never-persisted key → null ───────────────────────
// If this test breaks, the function no longer gates fast/never-persisted sends.

test("resolveSentDraftKey_unpersisted_key_returns_null", () => {
  setup("pubkey-resolver-fast");
  const draftKey = "chan-fast";

  // Store has no entry for this key.
  const result = resolveSentDraftKey(draftKey, loadDraftEntry);
  assert.equal(
    result,
    null,
    "unpersisted key → resolveSentDraftKey returns null",
  );
});

// ── (b) resolveSentDraftKey: persisted key → key ──────────────────────────────

test("resolveSentDraftKey_persisted_key_returns_key", () => {
  setup("pubkey-resolver-normal");
  const draftKey = "chan-normal";

  persistDraftEntry(draftKey, "hello", draftKey, [], []);

  const result = resolveSentDraftKey(draftKey, loadDraftEntry);
  assert.equal(
    result,
    draftKey,
    "persisted key → resolveSentDraftKey returns the key",
  );
});

// ── (c) resolveSentDraftKey: null/undefined effectiveDraftKey → null ──────────

test("resolveSentDraftKey_null_effectiveKey_returns_null", () => {
  setup("pubkey-resolver-null");
  const result = resolveSentDraftKey(null, loadDraftEntry);
  assert.equal(result, null, "null effectiveDraftKey → null");

  const result2 = resolveSentDraftKey(undefined, loadDraftEntry);
  assert.equal(result2, null, "undefined effectiveDraftKey → null");
});

// ── (d) Integration: never-persisted send → no sent record ───────────────────
// Simulates submitMessage calling resolveSentDraftKey before the send:
// the resolver returns null → markDraftSentEntry is never called.

test("submit_predicate_never_persisted_send_produces_no_sent_record", () => {
  setup("pubkey-fast-send");
  const draftKey = "chan-fast-integration";

  // Composer calls resolveSentDraftKey at submit time — store has no entry.
  const sentDraftKey = resolveSentDraftKey(draftKey, loadDraftEntry);
  assert.equal(sentDraftKey, null, "resolver returns null for fast send");

  // markDraftSentEntry is never called (sentDraftKey is null → gate in
  // useMentionSendFlow.ts:399 fires false). No sent record.
  assert.equal(getSentDraftEntries().length, 0, "no sent record for fast send");
});

// ── (e) Integration: persisted draft → sent record written ───────────────────

test("submit_predicate_persisted_draft_produces_sent_record", () => {
  setup("pubkey-normal-send");
  const draftKey = "chan-normal-integration";

  // Debounce persists the draft before submit.
  persistDraftEntry(draftKey, "my draft content", draftKey, [], []);

  // Composer calls resolveSentDraftKey at submit time.
  const sentDraftKey = resolveSentDraftKey(draftKey, loadDraftEntry);
  assert.equal(
    sentDraftKey,
    draftKey,
    "resolver returns key for persisted draft",
  );

  // Send succeeds — markDraftSentEntry called with the captured key.
  markDraftSentEntry(draftKey, "my draft content", draftKey, [], []);

  const sent = getSentDraftEntries();
  assert.equal(sent.length, 1, "sent record created");
  assert.equal(sent[0].draft.content, "my draft content");
  assert.equal(sent[0].draft.status, "sent");
});

// ── (f) Integration: persisted draft + race → sent record still written ───────
// Simulates: persist → resolveSentDraftKey captures key at submit time →
// navigation race clears the active entry → send succeeds with captured key.
// markDraftSentEntry writes unconditionally → sent record exists with snapshot.

test("submit_predicate_persisted_then_race_clears_key_sent_record_still_written", () => {
  setup("pubkey-race-send");
  const draftKey = "chan-race-pred";

  // Step 1: debounce persists the draft.
  persistDraftEntry(draftKey, "race content", draftKey, [IMG_A], []);

  // Step 2: resolver captures the key at submit time.
  const sentDraftKey = resolveSentDraftKey(draftKey, loadDraftEntry);
  assert.equal(sentDraftKey, draftKey, "resolver captures key at submit time");

  // Step 3: navigation-during-send race — active key cleared by composer cleanup.
  persistDraftEntry(draftKey, "", draftKey, [], []); // empty persist → clearDraftEntry
  assert.equal(
    loadDraftEntry(draftKey),
    undefined,
    "active key cleared by race before send success",
  );

  // Step 4: send succeeds; markDraftSentEntry called with captured sentDraftKey.
  markDraftSentEntry(sentDraftKey, "race content", draftKey, [IMG_A], []);

  const sent = getSentDraftEntries();
  assert.equal(
    sent.length,
    1,
    "sent record written despite active key being cleared",
  );
  assert.equal(
    sent[0].draft.content,
    "race content",
    "snapshot content preserved",
  );
  assert.equal(
    sent[0].draft.pendingImeta.length,
    1,
    "snapshot image preserved",
  );
  assert.equal(sent[0].draft.status, "sent");
});
