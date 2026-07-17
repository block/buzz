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
  renameDraftEntry,
  saveDraftEntry,
  subscribeToStore,
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

// ── markDraftSentEntry: clears active draft (no longer writes sent records) ──
// markDraftSentEntry now delegates to clearDraftEntry — it clears the active
// draft and writes nothing to the store. The sent-section UI was removed;
// these tests verify the new contract.

test("markDraftSent_clears_active_draft_and_writes_no_sent_record", () => {
  setup();
  persistDraftEntry("chan-1", "sent message content", "chan-1", [IMG_A], []);
  markDraftSentEntry("chan-1", "sent message content", "chan-1", [IMG_A], []);
  // Active key must be gone.
  assert.equal(
    loadDraftEntry("chan-1"),
    undefined,
    "active key must be cleared after markDraftSent",
  );
  // No sent records written.
  const sent = getSentDraftEntries();
  assert.equal(sent.length, 0, "no sent entries written");
  // All-entries also empty.
  assert.equal(getAllDraftEntries().length, 0, "store is empty after send");
});

test("markDraftSent_is_a_no_op_when_active_key_absent", () => {
  setup();
  // Call without any prior persistDraftEntry — simulates the race where the
  // active key was already cleared before markDraftSent ran.
  markDraftSentEntry("no-such-key", "content", "chan-x", [], []);
  assert.equal(
    loadDraftEntry("no-such-key"),
    undefined,
    "active key still absent",
  );
  // Still no sent record written.
  assert.equal(getSentDraftEntries().length, 0, "no sent entries");
  assert.equal(getAllDraftEntries().length, 0, "store still empty");
});

test("markDraftSent_new_active_draft_after_send_is_independent", () => {
  // After sending, a new draft typed in the same channel must appear in
  // getActiveDraftEntries() — markDraftSent must not affect it.
  setup("pubkey-coexist");
  persistDraftEntry("chan-X", "original draft", "chan-X", [], []);
  markDraftSentEntry("chan-X", "original draft", "chan-X", [], []);
  // New draft in the same channel.
  persistDraftEntry("chan-X", "new draft after send", "chan-X", [IMG_B], []);
  const active = getActiveDraftEntries();
  assert.equal(active.length, 1, "one active draft");
  assert.equal(active[0].draft.content, "new draft after send");
  assert.equal(active[0].draft.status, "active");
  // No sent records.
  assert.equal(getSentDraftEntries().length, 0, "no sent records");
});

test("getActiveDraftEntries_excludes_cleared_drafts", () => {
  setup("pubkey-active");
  persistDraftEntry("chan-active", "active draft", "chan-active", [], []);
  persistDraftEntry("chan-sent", "sent draft", "chan-sent", [], []);
  markDraftSentEntry("chan-sent", "sent draft", "chan-sent", [], []);
  const active = getActiveDraftEntries();
  assert.equal(active.length, 1, "only one active draft remains");
  assert.equal(active[0].key, "chan-active");
  assert.equal(active[0].draft.status, "active");
});

test("getSentDraftEntries_returns_empty_after_markDraftSent", () => {
  setup("pubkey-sent");
  persistDraftEntry("chan-active2", "still drafting", "chan-active2", [], []);
  persistDraftEntry("chan-sent2", "already sent", "chan-sent2", [], []);
  markDraftSentEntry("chan-sent2", "already sent", "chan-sent2", [], []);
  // markDraftSentEntry now just clears the active entry — no sent record.
  const sent = getSentDraftEntries();
  assert.equal(sent.length, 0, "no sent entries");
  // The remaining active draft is still there.
  const active = getActiveDraftEntries();
  assert.equal(active.length, 1, "one active draft");
  assert.equal(active[0].key, "chan-active2");
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

test("pre_status_sent_entry_is_dropped_on_read", () => {
  // Entries previously written with status "sent" (by the old markDraftSentEntry)
  // used a "sent:" key prefix. readStore now skips those keys entirely so legacy
  // sent records cannot resurface as ghost drafts.
  setup("pubkey-normalise");
  const oldSentEntry = {
    content: "a message I sent a while ago",
    selectionStart: 27,
    selectionEnd: 27,
    channelId: "chan-z",
    createdAt: "2025-09-01T00:00:00.000Z",
    updatedAt: "2025-09-01T00:00:00.000Z",
    pendingImeta: [],
    spoileredAttachmentUrls: [],
    status: "sent",
  };
  localStorage.setItem(
    "buzz-drafts.v1:pubkey-normalise",
    JSON.stringify({ "sent:chan-z:1725148800000-1": oldSentEntry }),
  );
  clearAllDrafts();
  initDraftStore("pubkey-normalise");
  // The sent: key is skipped — the entry must NOT appear as an active draft.
  const active = getActiveDraftEntries();
  assert.equal(active.length, 0, "old sent: entry is dropped, not promoted");
  assert.equal(getSentDraftEntries().length, 0, "no entries read as sent");
});

// ── renameDraftEntry storage-layer tests ─────────────────────────────────────────────────────────────────────────────────────────

function makeFullDraft(overrides = {}) {
  const now = new Date().toISOString();
  return {
    content: "Legacy content",
    selectionStart: 7,
    selectionEnd: 7,
    channelId: "chan-rename",
    createdAt: now,
    updatedAt: now,
    pendingImeta: [],
    spoileredAttachmentUrls: [],
    status: "active",
    ...overrides,
  };
}

function makeFullImeta(overrides = {}) {
  return {
    url: "https://cdn.example.com/img.jpg",
    sha256: "aabbccdd",
    size: 1024,
    type: "image/jpeg",
    uploaded: 999,
    dim: "800x600",
    blurhash: "LEHV6nWB2yk8pyo0adR*",
    thumb: undefined,
    duration: undefined,
    image: undefined,
    filename: "img.jpg",
    ...overrides,
  };
}

/** Read the raw persisted store blob for the current pubkey. */
function readRawStore(pubkey) {
  const raw = localStorage.getItem(`buzz-drafts.v1:${pubkey}`);
  return raw ? JSON.parse(raw) : {};
}

test("renameDraftEntry migrates: new key holds exact state, old key gone, one notify", () => {
  setup("pubkey-rename-basic");
  const draft = makeFullDraft();
  saveDraftEntry("inbox-reply:old111", draft);

  let notifyCount = 0;
  const unsub = subscribeToStore(() => {
    notifyCount++;
  });
  // Reset count after setup saves — only measure the rename call.
  notifyCount = 0;

  const result = renameDraftEntry("inbox-reply:old111", "thread:new222");

  unsub();

  assert.equal(result, "migrated");
  assert.equal(
    notifyCount,
    1,
    "exactly one subscriber notification on successful rename",
  );

  const moved = loadDraftEntry("thread:new222");
  assert.ok(moved, "canonical key must have the draft");
  // Single deepStrictEqual proves every field migrated without loss or mutation.
  assert.deepStrictEqual(
    moved,
    draft,
    "moved record must equal the seeded draft exactly",
  );

  assert.equal(
    loadDraftEntry("inbox-reply:old111"),
    undefined,
    "legacy key must be removed",
  );

  // Confirm persistence: localStorage must contain the canonical key, not the legacy key.
  const persisted = readRawStore("pubkey-rename-basic");
  assert.ok(
    "thread:new222" in persisted,
    "canonical key present in localStorage",
  );
  assert.ok(
    !("inbox-reply:old111" in persisted),
    "legacy key absent from localStorage",
  );
});

test("renameDraftEntry migrates with attachments: all BlobDescriptor fields preserved", () => {
  setup("pubkey-rename-attach");
  const img = makeFullImeta();
  const draft = makeFullDraft({
    pendingImeta: [img],
    spoileredAttachmentUrls: [img.url],
  });
  saveDraftEntry("inbox-reply:attach1", draft);

  const result = renameDraftEntry(
    "inbox-reply:attach1",
    "thread:attach-canonical",
  );
  assert.equal(result, "migrated");

  const moved = loadDraftEntry("thread:attach-canonical");
  assert.ok(moved);
  // deepStrictEqual confirms every BlobDescriptor optional field (dim, blurhash,
  // thumb, duration, image, filename) survived the round-trip.
  assert.deepStrictEqual(
    moved,
    draft,
    "moved record including all attachment metadata must equal the seeded draft",
  );
  assert.equal(
    loadDraftEntry("inbox-reply:attach1"),
    undefined,
    "legacy key removed",
  );
});

test("renameDraftEntry noop: old key absent, returns noop, no notify, no writes", () => {
  setup("pubkey-rename-noop");

  const storeBefore = readRawStore("pubkey-rename-noop");

  let notifyCount = 0;
  const unsub = subscribeToStore(() => {
    notifyCount++;
  });
  notifyCount = 0;

  const result = renameDraftEntry("inbox-reply:missing", "thread:target");

  unsub();
  assert.equal(result, "noop");
  assert.equal(notifyCount, 0, "no notification on noop");
  assert.equal(loadDraftEntry("thread:target"), undefined);

  // localStorage must be byte-identical before and after — no writes on noop.
  assert.deepStrictEqual(
    readRawStore("pubkey-rename-noop"),
    storeBefore,
    "localStorage must be unchanged on noop",
  );
});

test("renameDraftEntry oldKey === newKey: returns noop, no writes, no notify", () => {
  setup("pubkey-rename-same");
  saveDraftEntry("thread:same", makeFullDraft());

  const storeBefore = readRawStore("pubkey-rename-same");

  let notifyCount = 0;
  const unsub = subscribeToStore(() => {
    notifyCount++;
  });
  notifyCount = 0;

  const result = renameDraftEntry("thread:same", "thread:same");

  unsub();
  assert.equal(result, "noop");
  assert.equal(notifyCount, 0);

  // localStorage must be byte-identical before and after.
  assert.deepStrictEqual(
    readRawStore("pubkey-rename-same"),
    storeBefore,
    "localStorage must be unchanged on self-rename noop",
  );
});

test("renameDraftEntry collision on distinct content: both records preserved, no notify", () => {
  setup("pubkey-rename-collision");
  const legacy = makeFullDraft({ content: "Legacy draft" });
  const canonical = makeFullDraft({ content: "Canonical draft" });
  saveDraftEntry("inbox-reply:col1", legacy);
  saveDraftEntry("thread:col-target", canonical);

  // Snapshot both records before the rename attempt.
  const legacyBefore = loadDraftEntry("inbox-reply:col1");
  const canonicalBefore = loadDraftEntry("thread:col-target");
  const storeBefore = readRawStore("pubkey-rename-collision");

  let notifyCount = 0;
  const unsub = subscribeToStore(() => {
    notifyCount++;
  });
  notifyCount = 0;

  const result = renameDraftEntry("inbox-reply:col1", "thread:col-target");

  unsub();
  assert.equal(result, "collision");
  assert.equal(notifyCount, 0, "no notification on collision");

  // Deep-compare after to before: both records must be byte-identical to their pre-call snapshots.
  assert.deepStrictEqual(
    loadDraftEntry("inbox-reply:col1"),
    legacyBefore,
    "legacy record must be byte-identical after collision",
  );
  assert.deepStrictEqual(
    loadDraftEntry("thread:col-target"),
    canonicalBefore,
    "canonical record must be byte-identical after collision",
  );

  // localStorage must be byte-identical before and after — no writes on collision.
  assert.deepStrictEqual(
    readRawStore("pubkey-rename-collision"),
    storeBefore,
    "localStorage must be unchanged on collision",
  );
});

test("renameDraftEntry collision on optional attachment field only (blurhash differs): both preserved, no notify", () => {
  setup("pubkey-rename-collision-optional");
  const imgA = makeFullImeta({ blurhash: "HASH_A" });
  const imgB = makeFullImeta({ blurhash: "HASH_B" }); // differs only in blurhash
  const legacy = makeFullDraft({ pendingImeta: [imgA] });
  const canonical = makeFullDraft({ pendingImeta: [imgB] });
  saveDraftEntry("inbox-reply:opt1", legacy);
  saveDraftEntry("thread:opt-canonical", canonical);

  // Snapshot both records and the store before the rename attempt.
  const legacyBefore = loadDraftEntry("inbox-reply:opt1");
  const canonicalBefore = loadDraftEntry("thread:opt-canonical");
  const storeBefore = readRawStore("pubkey-rename-collision-optional");

  let notifyCount = 0;
  const unsub = subscribeToStore(() => {
    notifyCount++;
  });
  notifyCount = 0;

  const result = renameDraftEntry("inbox-reply:opt1", "thread:opt-canonical");

  unsub();
  assert.equal(
    result,
    "collision",
    "blurhash difference must trigger collision",
  );
  assert.equal(notifyCount, 0);

  // Deep-compare: both records must survive unchanged.
  assert.deepStrictEqual(
    loadDraftEntry("inbox-reply:opt1"),
    legacyBefore,
    "legacy record byte-identical after optional-field collision",
  );
  assert.deepStrictEqual(
    loadDraftEntry("thread:opt-canonical"),
    canonicalBefore,
    "canonical record byte-identical after optional-field collision",
  );

  // localStorage unchanged.
  assert.deepStrictEqual(
    readRawStore("pubkey-rename-collision-optional"),
    storeBefore,
    "localStorage must be unchanged on optional-field collision",
  );
});

test("renameDraftEntry identical records: legacy removed, canonical kept, one notify", () => {
  setup("pubkey-rename-identical");
  const now = new Date().toISOString();
  // Both records byte-identical across every field.
  const shared = makeFullDraft({ createdAt: now, updatedAt: now });
  saveDraftEntry("inbox-reply:ident", shared);
  saveDraftEntry("thread:ident-target", shared);

  let notifyCount = 0;
  const unsub = subscribeToStore(() => {
    notifyCount++;
  });
  notifyCount = 0;

  const result = renameDraftEntry("inbox-reply:ident", "thread:ident-target");

  unsub();
  assert.equal(result, "migrated", "identical records collapse to migrated");
  assert.equal(notifyCount, 1);
  assert.equal(
    loadDraftEntry("inbox-reply:ident"),
    undefined,
    "legacy key removed",
  );
  // deepStrictEqual on the surviving canonical record.
  assert.deepStrictEqual(
    loadDraftEntry("thread:ident-target"),
    shared,
    "canonical record must equal the seeded shared draft",
  );

  // Confirm localStorage reflects the final state.
  const persisted = readRawStore("pubkey-rename-identical");
  assert.ok(
    "thread:ident-target" in persisted,
    "canonical key in localStorage",
  );
  assert.ok(
    !("inbox-reply:ident" in persisted),
    "legacy key absent from localStorage",
  );
});

test("persist_draft_round_trips_stable_mention_refs_across_restart", () => {
  setup("mention-owner");
  const mentionRefs = [
    {
      displayName: "Agent Ada",
      pubkey: "abcdef1234",
      isAgent: true,
    },
    {
      displayName: "Pat Person",
      pubkey: "987654fedc",
      isAgent: false,
    },
  ];

  persistDraftEntry(
    "chan-mentions",
    "Hi @Agent Ada and @Pat Person",
    "chan-mentions",
    [],
    [],
    mentionRefs,
  );
  clearAllDrafts();
  initDraftStore("mention-owner");

  assert.deepEqual(loadDraftEntry("chan-mentions")?.mentionRefs, mentionRefs);
});

test("legacy_draft_without_mention_refs_migrates_to_empty_refs", () => {
  const storage = installFreshLocalStorage();
  clearAllDrafts();
  const now = new Date().toISOString();
  storage.setItem(
    "buzz-drafts.v1:legacy-owner",
    JSON.stringify({
      "chan-legacy": {
        content: "legacy @Ada",
        selectionStart: 11,
        selectionEnd: 11,
        channelId: "chan-legacy",
        createdAt: now,
        updatedAt: now,
        pendingImeta: [],
        spoileredAttachmentUrls: [],
        status: "active",
      },
    }),
  );

  initDraftStore("legacy-owner");
  assert.deepEqual(loadDraftEntry("chan-legacy")?.mentionRefs, []);
});

test("invalid_mention_ref_rejects_corrupt_draft", () => {
  const storage = installFreshLocalStorage();
  clearAllDrafts();
  const now = new Date().toISOString();
  storage.setItem(
    "buzz-drafts.v1:corrupt-mention-owner",
    JSON.stringify({
      "chan-corrupt": {
        content: "bad ref",
        selectionStart: 7,
        selectionEnd: 7,
        channelId: "chan-corrupt",
        createdAt: now,
        updatedAt: now,
        pendingImeta: [],
        mentionRefs: [{ displayName: "Ada", pubkey: 123, isAgent: true }],
        spoileredAttachmentUrls: [],
        status: "active",
      },
    }),
  );

  initDraftStore("corrupt-mention-owner");
  assert.equal(loadDraftEntry("chan-corrupt"), undefined);
});
