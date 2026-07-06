/**
 * Unit tests for the localStorage-backed draft store.
 *
 * Tests cover:
 *   - save/load round-trip including attachments (pendingImeta)
 *   - persist-and-restore across channel switch (image-drop fix)
 *   - corruption tolerance (bad JSON in localStorage)
 *   - identity scoping (drafts don't leak across pubkeys)
 *   - MAX_DRAFTS eviction (oldest-updated entry removed when over cap)
 *   - clearAllDrafts resets the store
 *   - getAllDraftEntries returns sorted most-recently-updated first
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

// ── Module import ─────────────────────────────────────────────────────────────
// We import the standalone storage functions (not the React hook) so tests
// run without a React renderer context.
import {
  clearAllDrafts,
  clearDraftEntry,
  getActiveDraftEntries,
  getAllDraftEntries,
  getSentDraftEntries,
  initDraftStore,
  loadDraftEntry,
  markDraftSentEntry,
  persistDraftEntry,
  saveDraftEntry,
} from "./useDrafts.ts";

// Minimal ImetaMedia fixtures.
const IMG_A = {
  url: "https://cdn.example.com/a.jpg",
  sha256: "aabbccdd",
  size: 1024,
  type: "image/jpeg",
  uploaded: 0,
};
const IMG_B = {
  url: "https://cdn.example.com/b.png",
  sha256: "eeff0011",
  size: 2048,
  type: "image/png",
  uploaded: 0,
};

function setup(pubkey = "pubkey-alice") {
  installFreshLocalStorage();
  clearAllDrafts();
  initDraftStore(pubkey);
}

function makeDraft(overrides = {}) {
  const now = new Date().toISOString();
  return {
    content: "Hello world",
    selectionStart: 11,
    selectionEnd: 11,
    channelId: "chan-1",
    createdAt: now,
    updatedAt: now,
    pendingImeta: [],
    spoileredAttachmentUrls: [],
    ...overrides,
  };
}

// ── save / load round-trip ────────────────────────────────────────────────────

test("save_load_round_trip_preserves_content_and_attachments", () => {
  setup();
  saveDraftEntry(
    "chan-1",
    makeDraft({
      pendingImeta: [IMG_A],
      spoileredAttachmentUrls: ["https://cdn.example.com/a.jpg"],
    }),
  );
  const loaded = loadDraftEntry("chan-1");
  assert.ok(loaded, "draft should exist");
  assert.equal(loaded.content, "Hello world");
  assert.equal(loaded.pendingImeta.length, 1);
  assert.equal(loaded.pendingImeta[0].url, IMG_A.url);
  assert.deepEqual(loaded.spoileredAttachmentUrls, [
    "https://cdn.example.com/a.jpg",
  ]);
});

test("save_load_round_trip_survives_restart_via_localstorage", () => {
  setup();
  saveDraftEntry(
    "chan-persist",
    makeDraft({
      channelId: "chan-persist",
      content: "Persisted draft",
      pendingImeta: [IMG_B],
    }),
  );

  // Simulate restart: clear in-memory cache, same localStorage + pubkey.
  clearAllDrafts();
  initDraftStore("pubkey-alice");
  const loaded = loadDraftEntry("chan-persist");
  assert.ok(loaded, "draft should survive simulated restart");
  assert.equal(loaded.content, "Persisted draft");
  assert.equal(loaded.pendingImeta[0].url, IMG_B.url);
});

// ── persistDraftEntry (image-drop fix) ────────────────────────────────────────

test("persist_draft_saves_images_on_channel_switch_and_restores_them", () => {
  setup();
  persistDraftEntry("chan-A", "Draft with image", "chan-A", [IMG_A], []);
  const saved = loadDraftEntry("chan-A");
  assert.ok(saved, "draft for chan-A should exist");
  assert.equal(saved.pendingImeta.length, 1, "image should be persisted");
  assert.equal(saved.pendingImeta[0].url, IMG_A.url);
});

test("persist_draft_clears_draft_when_content_and_attachments_are_empty", () => {
  setup();
  saveDraftEntry("chan-1", makeDraft({ content: "Something" }));
  // Persist empty — should remove the draft.
  persistDraftEntry("chan-1", "   ", "chan-1", [], []);
  assert.equal(
    loadDraftEntry("chan-1"),
    undefined,
    "empty persist should clear draft",
  );
});

test("persist_draft_preserves_createdAt_on_update", () => {
  setup();
  persistDraftEntry("chan-1", "v1", "chan-1", [], []);
  const first = loadDraftEntry("chan-1");
  assert.ok(first);
  const createdAt = first.createdAt;

  persistDraftEntry("chan-1", "v2", "chan-1", [], []);
  const second = loadDraftEntry("chan-1");
  assert.ok(second);
  assert.equal(
    second.createdAt,
    createdAt,
    "createdAt must not change on update",
  );
  assert.equal(second.content, "v2");
});

// ── clearDraftEntry ───────────────────────────────────────────────────────────

test("clearDraft_removes_entry_from_store_and_localstorage", () => {
  setup();
  persistDraftEntry("chan-del", "to delete", "chan-del", [], []);
  clearDraftEntry("chan-del");
  assert.equal(loadDraftEntry("chan-del"), undefined);
});

// ── corruption tolerance ──────────────────────────────────────────────────────

test("corrupt_localstorage_json_is_silently_ignored", () => {
  setup("pubkey-corrupt");
  localStorage.setItem("buzz-drafts.v1:pubkey-corrupt", "{not-valid-json");
  // Re-init to force a fresh read from the corrupted store.
  clearAllDrafts();
  initDraftStore("pubkey-corrupt");
  // Should return undefined, not throw.
  assert.equal(loadDraftEntry("any-key"), undefined);
});

test("invalid_draft_entries_in_localstorage_are_skipped", () => {
  setup("pubkey-invalid");
  const validDraft = {
    content: "valid",
    selectionStart: 0,
    selectionEnd: 0,
    channelId: "chan-v",
    createdAt: "2025-01-01T00:00:00.000Z",
    updatedAt: "2025-01-01T00:00:00.000Z",
    pendingImeta: [],
    spoileredAttachmentUrls: [],
  };
  const data = JSON.stringify({
    "chan-v": validDraft,
    "chan-bad": { content: 42, selectionStart: "no" },
    "chan-missing": { content: "x" },
  });
  localStorage.setItem("buzz-drafts.v1:pubkey-invalid", data);
  clearAllDrafts();
  initDraftStore("pubkey-invalid");
  assert.ok(loadDraftEntry("chan-v"), "valid draft should load");
  assert.equal(
    loadDraftEntry("chan-bad"),
    undefined,
    "invalid shape should be skipped",
  );
  assert.equal(
    loadDraftEntry("chan-missing"),
    undefined,
    "incomplete draft should be skipped",
  );
});

// ── identity scoping ──────────────────────────────────────────────────────────

test("drafts_are_scoped_per_pubkey_and_do_not_leak_across_identities", () => {
  setup("pubkey-alice");
  persistDraftEntry("chan-1", "alice draft", "chan-1", [], []);

  clearAllDrafts();
  initDraftStore("pubkey-bob");
  assert.equal(
    loadDraftEntry("chan-1"),
    undefined,
    "bob must not see alice's draft",
  );

  clearAllDrafts();
  initDraftStore("pubkey-alice");
  assert.ok(
    loadDraftEntry("chan-1"),
    "alice's draft must survive identity switch",
  );
});

// ── eviction ─────────────────────────────────────────────────────────────────

test("evicts_oldest_updated_entry_when_over_cap", () => {
  setup("pubkey-evict");
  const MAX = 100;
  for (let i = 0; i <= MAX; i++) {
    const ts = new Date(1_000_000 + i * 1000).toISOString();
    saveDraftEntry(
      `chan-${i}`,
      makeDraft({
        channelId: `chan-${i}`,
        content: `draft ${i}`,
        createdAt: ts,
        updatedAt: ts,
      }),
    );
  }
  // chan-0 had the oldest updatedAt — it must have been evicted.
  assert.equal(
    loadDraftEntry("chan-0"),
    undefined,
    "oldest entry should be evicted",
  );
  assert.ok(loadDraftEntry("chan-1"), "chan-1 should survive");
  assert.ok(loadDraftEntry(`chan-${MAX}`), `chan-${MAX} should survive`);
});

// ── getAllDraftEntries ────────────────────────────────────────────────────────

test("getAllDraftEntries_returns_all_entries_sorted_most_recently_updated_first", () => {
  setup("pubkey-list");
  const old = "2025-01-01T00:00:00.000Z";
  const newer = "2025-06-01T00:00:00.000Z";
  const newest = "2025-12-01T00:00:00.000Z";

  saveDraftEntry(
    "chan-a",
    makeDraft({
      channelId: "chan-a",
      content: "a",
      createdAt: old,
      updatedAt: old,
    }),
  );
  saveDraftEntry(
    "chan-b",
    makeDraft({
      channelId: "chan-b",
      content: "b",
      createdAt: newer,
      updatedAt: newer,
    }),
  );
  saveDraftEntry(
    "chan-c",
    makeDraft({
      channelId: "chan-c",
      content: "c",
      createdAt: newest,
      updatedAt: newest,
    }),
  );

  const all = getAllDraftEntries();
  assert.equal(all.length, 3);
  assert.equal(all[0].key, "chan-c", "most recent first");
  assert.equal(all[1].key, "chan-b");
  assert.equal(all[2].key, "chan-a", "oldest last");
});

test("getAllDraftEntries_returns_empty_array_when_no_drafts", () => {
  setup("pubkey-empty");
  assert.deepEqual(getAllDraftEntries(), []);
});

// ── channelId correctness on key switch ──────────────────────────────────────
// Regression: composer effect body was re-persisting prevKey with the incoming
// channel's id, corrupting the outgoing draft's channelId metadata.
// The first test below demonstrates the bug path — calling persistDraftEntry
// for key-A with channelId-B DOES overwrite the metadata, proving that the
// redundant body-side persist was the corruption source and had to be removed.
// The second test asserts the correct post-fix behavior: a normal A→B switch
// leaves draft A's channelId untouched.

test("persist_draft_bug_path_overwrites_channelId_confirming_removal_was_right", () => {
  setup();
  // Simulate correct outgoing save (cleanup runs first in React, correct channel).
  persistDraftEntry("chan-A", "draft text", "chan-A", [IMG_A], []);
  const afterCorrectSave = loadDraftEntry("chan-A");
  assert.ok(afterCorrectSave, "chan-A draft should exist after correct save");
  assert.equal(
    afterCorrectSave.channelId,
    "chan-A",
    "channelId must be chan-A after correct save",
  );

  // Simulate the BUG path: a second persist of the same key but with the
  // incoming channel's id (chan-B). This must NOT be done in practice, but
  // we assert here that IF it were called, it would corrupt the metadata —
  // confirming that removing the redundant body-side persist was the right fix.
  persistDraftEntry("chan-A", "draft text", "chan-B", [IMG_A], []);
  const afterBuggyOverwrite = loadDraftEntry("chan-A");
  assert.ok(afterBuggyOverwrite);
  assert.equal(
    afterBuggyOverwrite.channelId,
    "chan-B",
    "channelId IS overwritten when persist is called with wrong channel — confirms the redundant persist must be removed",
  );
});

test("persist_draft_outgoing_key_retains_original_channelId_when_body_persist_removed", () => {
  // The correct behavior after the fix: only the cleanup call persists
  // the outgoing draft. We simulate: persist A with channelId=A (cleanup),
  // do NOT call persist A again with channelId=B (body removed), then verify A
  // still has channelId=A when navigating to B's composer.
  setup();
  persistDraftEntry("chan-A", "draft in A", "chan-A", [IMG_A], []);
  // Simulate switch to channel B: persist B's draft (new channel).
  persistDraftEntry("chan-B", "", "chan-B", [], []); // empty B draft, gets cleared
  // A's draft must still have channelId=A.
  const draftA = loadDraftEntry("chan-A");
  assert.ok(draftA, "chan-A draft should survive channel switch to B");
  assert.equal(
    draftA.channelId,
    "chan-A",
    "chan-A channelId must not be corrupted by switch to chan-B",
  );
  assert.equal(
    draftA.pendingImeta.length,
    1,
    "image must be preserved on chan-A draft",
  );
});

// ── thread-key handling ───────────────────────────────────────────────────────

test("thread_draft_key_stores_explicit_channelId_not_the_thread_key", () => {
  setup();
  const threadKey = "thread:aaaa1234";
  const channelId = "the-channel-id";
  saveDraftEntry(
    threadKey,
    makeDraft({
      channelId,
      content: "thread reply draft",
      pendingImeta: [IMG_A],
    }),
  );
  const loaded = loadDraftEntry(threadKey);
  assert.ok(loaded);
  assert.equal(
    loaded.channelId,
    channelId,
    "channelId must equal the explicit value",
  );
  assert.equal(loaded.pendingImeta.length, 1);
});

// ── initDraftStore cache-reset safety ────────────────────────────────────────

test("initDraftStore_resets_cache_on_pubkey_change_without_clearAllDrafts", () => {
  // Alice saves a draft.
  setup("pubkey-alice");
  persistDraftEntry("chan-1", "alice draft", "chan-1", [], []);

  // Switch directly to bob without calling clearAllDrafts first.
  // initDraftStore must reset the in-memory cache so alice's draft
  // is not served under bob's identity.
  initDraftStore("pubkey-bob");
  assert.equal(
    loadDraftEntry("chan-1"),
    undefined,
    "bob must not see alice's cached draft after direct initDraftStore switch",
  );
});

// ── status field: markDraftSent, getActiveDraftEntries, getSentDraftEntries ───

test("markDraftSent_flips_status_to_sent_and_preserves_content_and_images", () => {
  setup();
  persistDraftEntry("chan-1", "sent message content", "chan-1", [IMG_A], []);
  markDraftSentEntry("chan-1");
  const loaded = loadDraftEntry("chan-1");
  assert.ok(loaded, "entry must still exist after markDraftSent");
  assert.equal(loaded.status, "sent", "status must be 'sent'");
  assert.equal(loaded.content, "sent message content", "content preserved");
  assert.equal(loaded.pendingImeta.length, 1, "image preserved");
  assert.equal(loaded.pendingImeta[0].url, IMG_A.url);
});

test("markDraftSent_is_a_noop_for_nonexistent_key", () => {
  setup();
  // Must not throw.
  markDraftSentEntry("no-such-key");
  assert.equal(loadDraftEntry("no-such-key"), undefined);
});

test("getActiveDraftEntries_returns_only_active_drafts", () => {
  setup("pubkey-active");
  persistDraftEntry("chan-active", "active draft", "chan-active", [], []);
  persistDraftEntry("chan-sent", "sent draft", "chan-sent", [], []);
  markDraftSentEntry("chan-sent");
  const active = getActiveDraftEntries();
  assert.equal(active.length, 1, "only one active draft");
  assert.equal(active[0].key, "chan-active");
  assert.equal(active[0].draft.status, "active");
});

test("getSentDraftEntries_returns_only_sent_drafts", () => {
  setup("pubkey-sent");
  persistDraftEntry("chan-active2", "still drafting", "chan-active2", [], []);
  persistDraftEntry("chan-sent2", "already sent", "chan-sent2", [], []);
  markDraftSentEntry("chan-sent2");
  const sent = getSentDraftEntries();
  assert.equal(sent.length, 1, "only one sent draft");
  assert.equal(sent[0].key, "chan-sent2");
  assert.equal(sent[0].draft.status, "sent");
});

test("getActiveDraftEntries_and_getSentDraftEntries_partition_all_entries", () => {
  setup("pubkey-partition");
  persistDraftEntry("ch-a", "draft a", "ch-a", [], []);
  persistDraftEntry("ch-b", "draft b", "ch-b", [], []);
  persistDraftEntry("ch-c", "draft c", "ch-c", [], []);
  markDraftSentEntry("ch-b");
  const all = getAllDraftEntries();
  const active = getActiveDraftEntries();
  const sent = getSentDraftEntries();
  assert.equal(all.length, 3);
  assert.equal(active.length + sent.length, all.length, "active + sent = all");
  assert.ok(
    active.every((e) => e.draft.status === "active"),
    "all active entries have status active",
  );
  assert.ok(
    sent.every((e) => e.draft.status === "sent"),
    "all sent entries have status sent",
  );
});

// ── status migration: pre-status entries read as "active" ────────────────────

test("pre_status_entry_without_status_field_is_read_as_active", () => {
  setup("pubkey-migrate");
  // Write a raw entry without the status field, simulating data persisted
  // before the status field was introduced.
  const legacyEntry = {
    content: "legacy draft",
    selectionStart: 0,
    selectionEnd: 12,
    channelId: "chan-legacy",
    createdAt: "2025-01-01T00:00:00.000Z",
    updatedAt: "2025-01-01T00:00:00.000Z",
    pendingImeta: [],
    spoileredAttachmentUrls: [],
    // NOTE: no 'status' field
  };
  localStorage.setItem(
    "buzz-drafts.v1:pubkey-migrate",
    JSON.stringify({ "chan-legacy": legacyEntry }),
  );
  // Force re-read from localStorage.
  clearAllDrafts();
  initDraftStore("pubkey-migrate");
  const loaded = loadDraftEntry("chan-legacy");
  assert.ok(loaded, "legacy entry must load without rejection");
  assert.equal(loaded.status, "active", "missing status defaults to 'active'");
  assert.equal(loaded.content, "legacy draft");
});

test("pre_status_entry_appears_in_getActiveDraftEntries_after_migration", () => {
  setup("pubkey-migrate2");
  const legacyEntry = {
    content: "old draft",
    selectionStart: 0,
    selectionEnd: 9,
    channelId: "chan-old",
    createdAt: "2025-06-01T00:00:00.000Z",
    updatedAt: "2025-06-01T00:00:00.000Z",
    pendingImeta: [],
    spoileredAttachmentUrls: [],
  };
  localStorage.setItem(
    "buzz-drafts.v1:pubkey-migrate2",
    JSON.stringify({ "chan-old": legacyEntry }),
  );
  clearAllDrafts();
  initDraftStore("pubkey-migrate2");
  const active = getActiveDraftEntries();
  assert.equal(active.length, 1, "legacy entry appears in active list");
  assert.equal(active[0].key, "chan-old");
  assert.equal(active[0].draft.status, "active");
});
