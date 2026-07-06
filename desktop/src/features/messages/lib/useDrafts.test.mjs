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
// markDraftSentEntry snapshots the draft into a distinct `sent:<key>:<ts>` key
// and removes the original active key so composer cleanup and new drafts are
// never affected by the sent record's lifecycle.

test("markDraftSent_writes_sent_record_under_distinct_key_and_removes_active_key", () => {
  setup();
  persistDraftEntry("chan-1", "sent message content", "chan-1", [IMG_A], []);
  markDraftSentEntry("chan-1", "sent message content", "chan-1", [IMG_A], []);
  // Active key must be gone.
  assert.equal(
    loadDraftEntry("chan-1"),
    undefined,
    "active key must be cleared after markDraftSent",
  );
  // Sent record must exist under a sent: key.
  const sent = getSentDraftEntries();
  assert.equal(sent.length, 1, "one sent entry must exist");
  assert.equal(sent[0].draft.status, "sent", "status must be 'sent'");
  assert.equal(
    sent[0].draft.content,
    "sent message content",
    "content preserved",
  );
  assert.equal(sent[0].draft.pendingImeta.length, 1, "image preserved");
  assert.equal(sent[0].draft.pendingImeta[0].url, IMG_A.url);
  assert.ok(
    sent[0].key.startsWith("sent:chan-1:"),
    "sent key must have sent: prefix",
  );
});

test("markDraftSent_writes_sent_record_even_when_active_key_already_cleared", () => {
  // The never-persisted boundary is enforced at the call site (sentDraftKey
  // is only set when a draft was actually persisted). This function writes
  // unconditionally so a navigation-during-send race cannot cause data loss:
  // if the active key was already cleared before send success, the snapshot
  // content still produces a sent record (createdAt falls back to now).
  setup();
  // Call without any prior persistDraftEntry — simulates the race where the
  // active key was deleted by a composer cleanup before markDraftSent ran.
  markDraftSentEntry("no-such-key", "content", "chan-x", [], []);
  assert.equal(
    loadDraftEntry("no-such-key"),
    undefined,
    "active key still absent",
  );
  const sent = getSentDraftEntries();
  assert.equal(
    sent.length,
    1,
    "sent record is written even without a live active key",
  );
  assert.equal(sent[0].draft.content, "content");
  assert.equal(sent[0].draft.status, "sent");
  assert.ok(
    sent[0].key.startsWith("sent:no-such-key:"),
    "sent key has correct prefix",
  );
});

test("markDraftSent_send_then_cleanup_preserves_sent_record", () => {
  // Simulate the full composer lifecycle:
  // 1. Draft exists on key A.
  // 2. User sends -> markDraftSent(A) snapshots under sent:A:ts and clears A.
  // 3. Composer cleanup calls persistDraft(A, "", ...) -> clearDraftEntry(A).
  // The sent record under sent:A:ts must still exist after step 3.
  setup();
  persistDraftEntry("chan-A", "my draft", "chan-A", [IMG_A], []);
  markDraftSentEntry("chan-A", "my draft", "chan-A", [IMG_A], []);
  // Simulate composer cleanup: empty persist on the now-absent active key.
  persistDraftEntry("chan-A", "", "chan-A", [], []);
  const sent = getSentDraftEntries();
  assert.equal(sent.length, 1, "sent record must survive composer cleanup");
  assert.equal(sent[0].draft.content, "my draft");
});

test("markDraftSent_navigation_during_async_send_still_creates_sent_record", () => {
  // Regression test for the async-send/navigation race (Thufir Pass-2 finding):
  // 1. Persisted draft A exists at submit time.
  // 2. Composer clears the editor (clearContent) then awaits onSend.
  // 3. While onSend is in flight, user switches channel. MessageComposer
  //    cleanup runs persistDraftEntry(A, empty) -> clearDraftEntry(A) — active
  //    key is gone before send success.
  // 4. Send succeeds; markDraftSentEntry(A, savedContent, ...) runs.
  // The sent record MUST still be written from the passed-in snapshot.
  setup();
  persistDraftEntry("chan-race", "race draft", "chan-race", [IMG_A], []);
  // Simulate step 3: active key cleared by navigation-during-send cleanup.
  persistDraftEntry("chan-race", "", "chan-race", [], []);
  assert.equal(
    loadDraftEntry("chan-race"),
    undefined,
    "active key should be cleared (simulating race)",
  );
  // Simulate step 4: send succeeds, mark sent with full snapshot.
  markDraftSentEntry("chan-race", "race draft", "chan-race", [IMG_A], []);
  const sent = getSentDraftEntries();
  assert.equal(
    sent.length,
    1,
    "sent record must be written despite active key being gone",
  );
  assert.equal(
    sent[0].draft.content,
    "race draft",
    "snapshot content preserved",
  );
  assert.equal(
    sent[0].draft.pendingImeta.length,
    1,
    "snapshot image preserved",
  );
  assert.equal(sent[0].draft.status, "sent");
});

test("markDraftSent_new_active_draft_after_send_is_independent", () => {
  // After sending, a new draft typed in the same channel must appear in
  // getActiveDraftEntries() as active, and the sent record must remain in
  // getSentDraftEntries() -- they coexist under distinct keys.
  setup("pubkey-coexist");
  persistDraftEntry("chan-X", "original draft", "chan-X", [], []);
  markDraftSentEntry("chan-X", "original draft", "chan-X", [], []);
  // New draft in the same channel.
  persistDraftEntry("chan-X", "new draft after send", "chan-X", [IMG_B], []);
  const active = getActiveDraftEntries();
  const sent = getSentDraftEntries();
  assert.equal(active.length, 1, "one active draft");
  assert.equal(active[0].draft.content, "new draft after send");
  assert.equal(active[0].draft.status, "active");
  assert.equal(sent.length, 1, "one sent record");
  assert.equal(sent[0].draft.content, "original draft");
  assert.equal(sent[0].draft.status, "sent");
});

test("markDraftSent_double_send_in_same_channel_creates_two_distinct_sent_records", () => {
  // Sending twice from the same channel must produce two independent sent
  // records -- the timestamp suffix prevents key collision.
  setup("pubkey-double-send");
  persistDraftEntry("chan-Y", "first draft", "chan-Y", [], []);
  markDraftSentEntry("chan-Y", "first draft", "chan-Y", [], []);
  // Second draft in the same channel.
  persistDraftEntry("chan-Y", "second draft", "chan-Y", [], []);
  markDraftSentEntry("chan-Y", "second draft", "chan-Y", [], []);
  const sent = getSentDraftEntries();
  assert.equal(sent.length, 2, "two distinct sent records");
  const contents = sent.map((e) => e.draft.content).sort();
  assert.deepEqual(contents, ["first draft", "second draft"]);
  const keys = sent.map((e) => e.key);
  assert.notEqual(keys[0], keys[1], "sent keys must be distinct");
});

test("getActiveDraftEntries_returns_only_active_drafts", () => {
  setup("pubkey-active");
  persistDraftEntry("chan-active", "active draft", "chan-active", [], []);
  persistDraftEntry("chan-sent", "sent draft", "chan-sent", [], []);
  markDraftSentEntry("chan-sent", "sent draft", "chan-sent", [], []);
  const active = getActiveDraftEntries();
  assert.equal(active.length, 1, "only one active draft");
  assert.equal(active[0].key, "chan-active");
  assert.equal(active[0].draft.status, "active");
});

test("getSentDraftEntries_returns_only_sent_drafts", () => {
  setup("pubkey-sent");
  persistDraftEntry("chan-active2", "still drafting", "chan-active2", [], []);
  persistDraftEntry("chan-sent2", "already sent", "chan-sent2", [], []);
  markDraftSentEntry("chan-sent2", "already sent", "chan-sent2", [], []);
  const sent = getSentDraftEntries();
  assert.equal(sent.length, 1, "only one sent draft");
  assert.ok(sent[0].key.startsWith("sent:chan-sent2:"), "sent key has prefix");
  assert.equal(sent[0].draft.status, "sent");
});

test("getActiveDraftEntries_and_getSentDraftEntries_partition_all_entries", () => {
  setup("pubkey-partition");
  persistDraftEntry("ch-a", "draft a", "ch-a", [], []);
  persistDraftEntry("ch-b", "draft b", "ch-b", [], []);
  persistDraftEntry("ch-c", "draft c", "ch-c", [], []);
  markDraftSentEntry("ch-b", "draft b", "ch-b", [], []);
  const all = getAllDraftEntries();
  const active = getActiveDraftEntries();
  const sent = getSentDraftEntries();
  // ch-a, ch-c still active; sent:ch-b:ts is the sent record.
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
