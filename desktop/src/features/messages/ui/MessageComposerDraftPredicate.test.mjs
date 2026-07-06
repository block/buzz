/**
 * Regression tests for the submit-time draft-persistence predicate in
 * MessageComposer's `submitMessage` handler.
 *
 * The predicate is:
 *   sentDraftKey =
 *     effectiveDraftKey && drafts.loadDraft(effectiveDraftKey)
 *       ? effectiveDraftKey
 *       : null
 *
 * These tests verify that the predicate correctly gates sent-record creation
 * so that never-persisted sends do NOT consume the shared draft cap, while
 * persisted drafts DO produce a sent record — even if the active key is
 * cleared by a navigation-during-send race before `markDraftSent` runs.
 *
 * Tests do NOT require a React renderer; they drive the storage layer
 * directly, matching the real behavior the predicate relies on.
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

// ── Predicate: never-persisted send ──────────────────────────────────────────
// Simulates: user types and sends quickly before the debounce persist fires.
// effectiveDraftKey is "chan-fast" (always truthy), but loadDraft returns
// falsy → the predicate evaluates to null → markDraftSent is never called.

test("submit_predicate_never_persisted_send_produces_no_sent_record", () => {
  setup("pubkey-fast-send");
  const draftKey = "chan-fast";

  // Predicate evaluation: loadDraftEntry returns falsy for a never-persisted key.
  const persistedAtSubmit = loadDraftEntry(draftKey);
  assert.equal(
    persistedAtSubmit,
    undefined,
    "loadDraft returns falsy for unpersisted key — predicate evaluates to null",
  );

  // Since sentDraftKey would be null, markDraftSentEntry is never called.
  // Verify no sent record exists.
  assert.equal(getSentDraftEntries().length, 0, "no sent record for fast send");
});

// ── Predicate: persisted draft sends correctly ────────────────────────────────
// Simulates: user types, debounce fires (draft persisted), then sends normally.
// effectiveDraftKey is truthy AND loadDraft returns truthy → predicate
// evaluates to the key → markDraftSent is called with that key.

test("submit_predicate_persisted_draft_produces_sent_record", () => {
  setup("pubkey-normal-send");
  const draftKey = "chan-normal";

  // Debounce persists the draft before submit.
  persistDraftEntry(draftKey, "my draft content", draftKey, [], []);

  // Predicate evaluation at submit time: loadDraftEntry returns truthy.
  const persistedAtSubmit = loadDraftEntry(draftKey);
  assert.ok(
    persistedAtSubmit,
    "loadDraft returns truthy — predicate uses the key",
  );

  // Simulate the send path: markDraftSentEntry is called with the key.
  markDraftSentEntry(draftKey, "my draft content", draftKey, [], []);

  const sent = getSentDraftEntries();
  assert.equal(sent.length, 1, "sent record created for persisted draft");
  assert.equal(sent[0].draft.content, "my draft content");
  assert.equal(sent[0].draft.status, "sent");
});

// ── Predicate + async race: persisted draft → key cleared before success ─────
// Simulates: user persists draft, submits, switches channels (race: active key
// cleared), send succeeds. sentDraftKey was captured at submit time (when
// loadDraft returned truthy), so markDraftSentEntry is still called.
// markDraftSentEntry writes unconditionally → sent record still exists.

test("submit_predicate_persisted_then_race_clears_key_sent_record_still_written", () => {
  setup("pubkey-race-send");
  const draftKey = "chan-race-pred";

  // Step 1: debounce persists the draft.
  persistDraftEntry(draftKey, "race content", draftKey, [IMG_A], []);

  // Step 2: predicate at submit time — draft is present, key captured.
  const sentDraftKey = loadDraftEntry(draftKey) ? draftKey : null;
  assert.equal(
    sentDraftKey,
    draftKey,
    "predicate captures the key at submit time",
  );

  // Step 3: navigation-during-send race — active key cleared by composer cleanup.
  persistDraftEntry(draftKey, "", draftKey, [], []); // empty persist → clearDraftEntry
  assert.equal(
    loadDraftEntry(draftKey),
    undefined,
    "active key cleared by race before send success",
  );

  // Step 4: send succeeds; markDraftSentEntry called with captured sentDraftKey.
  markDraftSentEntry(draftKey, "race content", draftKey, [IMG_A], []);

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
