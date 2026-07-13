import assert from "node:assert/strict";
import test from "node:test";

function makeLocalStorage() {
  const values = new Map();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
}

globalThis.window = {
  localStorage: makeLocalStorage(),
  setTimeout,
  clearTimeout,
};
Object.defineProperty(globalThis, "localStorage", {
  get: () => globalThis.window.localStorage,
});

import { DraftSyncManager } from "./draftSync.ts";
import {
  clearAllDrafts,
  initDraftStore,
  loadDraftEntry,
  removeRemoteDraftEntry,
  saveDraftEntry,
} from "./useDrafts.ts";

const pubkey = "a".repeat(64);
const channelA = "550e8400-e29b-41d4-a716-446655440000";
const channelB = "550e8400-e29b-41d4-a716-446655440001";

function wrapped({ id, createdAt, address, channelId, content }) {
  return {
    id,
    created_at: createdAt,
    kind: 31234,
    pubkey,
    content,
    sig: "",
    tags: [
      ["d", address],
      ["h", channelId],
      ["k", "9"],
    ],
  };
}

function payload(channelId, content) {
  return JSON.stringify({
    kind: 9,
    created_at: 1,
    pubkey,
    content,
    tags: [["h", channelId]],
  });
}

function setup() {
  globalThis.window.localStorage = makeLocalStorage();
  clearAllDrafts();
  initDraftStore(pubkey, "wss://relay.example");
}

test("test_two_addresses_out_of_order_each_merge_current_head", async () => {
  setup();
  const events = [
    wrapped({
      id: "new-a",
      createdAt: 20,
      address: "address-a",
      channelId: channelA,
      content: "cipher-a",
    }),
    wrapped({
      id: "old-b",
      createdAt: 10,
      address: "address-b",
      channelId: channelB,
      content: "cipher-b",
    }),
  ];
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    decrypt: async (cipher) =>
      cipher === "cipher-a" ? payload(channelA, "A") : payload(channelB, "B"),
    deriveAddress: async (draftKey) =>
      draftKey === channelA ? "address-a" : "address-b",
    fetchEvents: async () => events,
  });

  await manager.fetchAllOwnDrafts();
  assert.equal(loadDraftEntry(channelA)?.content, "A");
  assert.equal(loadDraftEntry(channelB)?.content, "B");
});

test("test_older_event_for_same_address_does_not_replace_newer_draft", async () => {
  setup();
  const newer = wrapped({
    id: "newer",
    createdAt: 2,
    address: "address-a",
    channelId: channelA,
    content: "new-cipher",
  });
  const older = wrapped({
    id: "older",
    createdAt: 1,
    address: "address-a",
    channelId: channelA,
    content: "old-cipher",
  });
  let events = [newer];
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    decrypt: async (cipher) =>
      payload(channelA, cipher === "new-cipher" ? "new draft" : "old draft"),
    deriveAddress: async () => "address-a",
    fetchEvents: async () => events,
  });

  await manager.fetchAllOwnDrafts();
  events = [older];
  await manager.fetchAllOwnDrafts();
  assert.equal(loadDraftEntry(channelA)?.content, "new draft");
});

test("test_decrypted_context_with_mismatched_address_is_rejected", async () => {
  setup();
  const address = "address-from-event";
  const mismatched = wrapped({
    id: "mismatched-address",
    createdAt: 2,
    address,
    channelId: channelA,
    content: "mismatched-cipher",
  });
  const valid = wrapped({
    id: "valid-address",
    createdAt: 1,
    address,
    channelId: channelA,
    content: "valid-cipher",
  });
  let events = [mismatched];
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    decrypt: async (cipher) =>
      cipher === "mismatched-cipher"
        ? JSON.stringify({
            kind: 9,
            created_at: 1,
            pubkey,
            content: "must not restore",
            tags: [
              ["h", channelA],
              ["e", "other-root", "", "reply"],
            ],
          })
        : payload(channelA, "valid draft"),
    deriveAddress: async (draftKey) =>
      draftKey === channelA ? address : "address-derived-from-context",
    fetchEvents: async () => events,
  });

  await manager.fetchAllOwnDrafts();
  assert.equal(loadDraftEntry(channelA), undefined);
  events = [valid];
  await manager.fetchAllOwnDrafts();
  assert.equal(loadDraftEntry(channelA)?.content, "valid draft");
});

test("test_tombstone_failure_sidecar_suppresses_remote_resurrection", async () => {
  setup();
  const remote = wrapped({
    id: "remote",
    createdAt: 1,
    address: "address-a",
    channelId: channelA,
    content: "cipher",
  });
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    deriveAddress: async () => "address-a",
    sign: async (input) => ({
      ...remote,
      id: "tombstone",
      created_at: 2,
      content: input.content,
    }),
    publishEvent: async () => {
      throw new Error("offline");
    },
    decrypt: async () => payload(channelA, "must not return"),
    fetchEvents: async () => [remote],
  });

  await manager.queueDeletion(channelA, channelA);
  await manager.fetchAllOwnDrafts();
  assert.equal(loadDraftEntry(channelA), undefined);
  assert.match(
    localStorage.getItem(`buzz-draft-sync.v1:wss://relay.example:${pubkey}`) ??
      "",
    /address-a/,
  );
});

test("test_remote_tombstone_blocks_stale_cleanup_publish", async () => {
  setup();
  const remote = wrapped({
    id: "remote-draft",
    createdAt: 1,
    address: "address-a",
    channelId: channelA,
    content: "cipher",
  });
  const tombstone = wrapped({
    id: "remote-tombstone",
    createdAt: 2,
    address: "address-a",
    channelId: channelA,
    content: "",
  });
  let events = [remote];
  const published = [];
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    decrypt: async () => payload(channelA, "draft"),
    deriveAddress: async () => "address-a",
    encrypt: async (content) => content,
    fetchEvents: async () => events,
    sign: async (input) => ({
      id: "stale-cleanup-publish",
      created_at: input.createdAt ?? 0,
      kind: input.kind,
      pubkey,
      content: input.content,
      sig: "",
      tags: input.tags,
    }),
    publishEvent: async (event) => published.push(event),
  });

  await manager.fetchAllOwnDrafts();
  events = [tombstone];
  await manager.fetchAllOwnDrafts();

  // Models the mounted composer's stale cleanup after the remote delete. The
  // unconditional tombstone abort must remove this local write without
  // publishing it back to the relay.
  const stale = draft(channelA, "stale cleanup content");
  saveDraftEntry(channelA, stale);
  manager.queuePublish(channelA, stale);
  await manager.flushPublishes();
  await manager.destroy();

  assert.deepEqual(published, []);
  assert.equal(loadDraftEntry(channelA), undefined);
});

test("test_remote_tombstone_removes_known_draft", async () => {
  setup();
  const remote = wrapped({
    id: "remote",
    createdAt: 1,
    address: "address-a",
    channelId: channelA,
    content: "cipher",
  });
  const tombstone = wrapped({
    id: "tombstone",
    createdAt: 2,
    address: "address-a",
    channelId: channelA,
    content: "",
  });
  let events = [remote];
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    decrypt: async () => payload(channelA, "draft"),
    deriveAddress: async () => "address-a",
    fetchEvents: async () => events,
  });

  await manager.fetchAllOwnDrafts();
  assert.equal(loadDraftEntry(channelA)?.content, "draft");
  events = [tombstone];
  await manager.fetchAllOwnDrafts();
  assert.equal(loadDraftEntry(channelA), undefined);
});

function draft(channelId, content, pendingImeta = []) {
  return {
    channelId,
    content,
    selectionStart: content.length,
    selectionEnd: content.length,
    createdAt: new Date(0).toISOString(),
    updatedAt: new Date(0).toISOString(),
    pendingImeta,
    spoileredAttachmentUrls: [],
    status: "active",
  };
}

test("test_pending_local_publish_observing_tombstone_does_not_resurrect_draft", async () => {
  setup();
  const tombstone = wrapped({
    id: "tombstone",
    createdAt: 2,
    address: "address-a",
    channelId: channelA,
    content: "",
  });
  const published = [];
  const local = draft(channelA, "local edit");
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    deriveAddress: async () => "address-a",
    fetchEvents: async () => [tombstone],
    sign: async (input) => ({ ...tombstone, ...input, id: "signed" }),
    publishEvent: async (event) => published.push(event),
  });

  saveDraftEntry(channelA, local);
  manager.queuePublish(channelA, local);
  await manager.destroy();

  assert.equal(published.length, 0);
  assert.equal(loadDraftEntry(channelA), undefined);
});

test("test_remote_update_does_not_overwrite_pending_local_edit", async () => {
  setup();
  const remote = wrapped({
    id: "remote",
    createdAt: 2,
    address: "address-a",
    channelId: channelA,
    content: "remote-cipher",
  });
  const local = draft(channelA, "local edit");
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    decrypt: async () => payload(channelA, "remote edit"),
    deriveAddress: async () => "address-a",
    fetchEvents: async () => [remote],
  });

  saveDraftEntry(channelA, local);
  manager.queuePublish(channelA, local);
  await manager.fetchAllOwnDrafts();

  assert.equal(loadDraftEntry(channelA)?.content, "local edit");
});

test("test_deleting_one_draft_does_not_strand_another_pending_publish", async () => {
  setup();
  const published = [];
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    deriveAddress: async (draftKey) =>
      draftKey === channelA ? "address-a" : "address-b",
    fetchEvents: async () => [],
    encrypt: async () => "cipher",
    sign: async (input) => ({
      id: `signed-${input.content || "tombstone"}`,
      created_at: input.createdAt ?? 0,
      kind: input.kind,
      pubkey,
      content: input.content,
      sig: "",
      tags: input.tags,
    }),
    publishEvent: async (event) => published.push(event),
  });

  manager.queuePublish(channelA, draft(channelA, "draft A"));
  await manager.queueDeletion(channelB, channelB);
  await manager.destroy();

  assert.ok(published.some((event) => event.content === "cipher"));
});

test("test_tombstone_rebases_after_future_remote_head", async () => {
  setup();
  const future = Math.floor(Date.now() / 1_000) + 10_000;
  const remote = wrapped({
    id: "remote",
    createdAt: future,
    address: "address-a",
    channelId: channelA,
    content: "cipher",
  });
  const signed = [];
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    decrypt: async () => payload(channelA, "draft"),
    deriveAddress: async () => "address-a",
    fetchEvents: async () => [remote],
    sign: async (input) => {
      signed.push(input);
      return { ...remote, ...input, id: "tombstone" };
    },
    publishEvent: async () => {},
  });

  await manager.fetchAllOwnDrafts();
  await manager.queueDeletion(channelA, channelA);

  assert.equal(signed[0].content, "");
  assert.equal(signed[0].createdAt, future + 1);
});

test("test_unuploaded_attachment_cancels_stale_text_publish", async () => {
  setup();
  const published = [];
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    deriveAddress: async () => "address-a",
    fetchEvents: async () => [],
    encrypt: async () => "cipher",
    sign: async (input) => ({
      id: "signed",
      created_at: input.createdAt ?? 0,
      kind: input.kind,
      pubkey,
      content: input.content,
      sig: "",
      tags: input.tags,
    }),
    publishEvent: async (event) => published.push(event),
  });

  manager.queuePublish(channelA, draft(channelA, "text"));
  manager.queuePublish(
    channelA,
    draft(channelA, "text", [{ uploaded: false }]),
  );
  await manager.destroy();

  assert.equal(published.length, 0);
});

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((complete, fail) => {
    resolve = complete;
    reject = fail;
  });
  return { promise, resolve, reject };
}

test("test_newer_edit_during_inflight_publish_survives", async () => {
  setup();
  const published = [];
  const firstPublish = deferred();
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    deriveAddress: async () => "address-a",
    encrypt: async (content) => content,
    fetchEvents: async () => [],
    sign: async (input) => ({
      id: `signed-${published.length}`,
      created_at: input.createdAt ?? 0,
      kind: input.kind,
      pubkey,
      content: input.content,
      sig: "",
      tags: input.tags,
    }),
    publishEvent: async (event) => {
      published.push(event);
      if (published.length === 1) await firstPublish.promise;
    },
  });

  manager.queuePublish(channelA, draft(channelA, "older edit"));
  const flush = manager.flushPublishes();
  while (published.length === 0) await Promise.resolve();
  manager.queuePublish(channelA, draft(channelA, "newer edit"));
  firstPublish.resolve();
  await flush;
  await manager.destroy();

  assert.equal(published.length, 2);
  assert.match(published[1].content, /newer edit/);
});

test("test_deletion_during_inflight_publish_wins", async () => {
  setup();
  const published = [];
  const firstPublish = deferred();
  const tombstonePublished = deferred();
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    deriveAddress: async () => "address-a",
    encrypt: async (content) => content,
    fetchEvents: async () => [],
    sign: async (input) => ({
      id: `signed-${published.length}`,
      created_at: input.createdAt ?? 0,
      kind: input.kind,
      pubkey,
      content: input.content,
      sig: "",
      tags: input.tags,
    }),
    publishEvent: async (event) => {
      published.push(event);
      if (published.length === 1) await firstPublish.promise;
      if (event.content === "") tombstonePublished.resolve();
    },
  });

  const local = draft(channelA, "draft to delete");
  saveDraftEntry(channelA, local);
  manager.queuePublish(channelA, local);
  const flush = manager.flushPublishes();
  while (published.length === 0) await Promise.resolve();
  const deletion = manager.queueDeletion(channelA, channelA);
  await tombstonePublished.promise;
  removeRemoteDraftEntry(channelA);
  firstPublish.resolve();
  await flush;
  await deletion;
  await manager.destroy();

  const draftEvent = published.find((event) => event.content !== "");
  const rebasedTombstone = published.find(
    (event) => event.content === "" && event.created_at > draftEvent.created_at,
  );
  assert.ok(rebasedTombstone);
  assert.equal(loadDraftEntry(channelA), undefined);
});

test("test_stale_tombstone_completion_preserves_rebased_delete", async () => {
  setup();
  const published = [];
  const draftPublish = deferred();
  const staleTombstone = deferred();
  const rebasedTombstone = deferred();
  const rebasedTombstoneStarted = deferred();
  const manager = new DraftSyncManager(pubkey, "wss://relay.example", {
    deriveAddress: async () => "address-a",
    encrypt: async (content) => content,
    fetchEvents: async () => [],
    sign: async (input) => ({
      id: `signed-${published.length}`,
      created_at:
        input.content === "" && published.length === 1
          ? (input.createdAt ?? 0) - 1
          : (input.createdAt ?? 0),
      kind: input.kind,
      pubkey,
      content: input.content,
      sig: "",
      tags: input.tags,
    }),
    publishEvent: async (event) => {
      const publishIndex = published.push(event);
      if (publishIndex === 1) await draftPublish.promise;
      if (publishIndex === 2) await staleTombstone.promise;
      if (publishIndex === 3) {
        rebasedTombstoneStarted.resolve();
        await rebasedTombstone.promise;
      }
    },
  });

  const local = draft(channelA, "draft to delete");
  saveDraftEntry(channelA, local);
  manager.queuePublish(channelA, local);
  const flush = manager.flushPublishes();
  while (published.length === 0) await Promise.resolve();
  const deletion = manager.queueDeletion(channelA, channelA);
  removeRemoteDraftEntry(channelA);
  while (published.length < 2) await Promise.resolve();
  draftPublish.resolve();
  const draftEvent = published[0];
  staleTombstone.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
  await rebasedTombstoneStarted.promise;
  rebasedTombstone.reject(new Error("offline"));
  await flush;
  await deletion;

  const retried = [];
  const retryManager = new DraftSyncManager(pubkey, "wss://relay.example", {
    deriveAddress: async () => "address-a",
    publishEvent: async (event) => retried.push(event),
    sign: async (input) => ({
      id: "retry",
      created_at: input.createdAt ?? 0,
      kind: input.kind,
      pubkey,
      content: input.content,
      sig: "",
      tags: input.tags,
    }),
  });
  retryManager.start();
  await Promise.resolve();
  await retryManager.destroy();
  await manager.destroy();

  assert.ok(retried.some((event) => event.content === ""));
  assert.ok(retried.every((event) => event.created_at > draftEvent.created_at));
});
