/**
 * Unit tests for migrateInboxReplyDraft().
 *
 * Uses real getThreadReference and getChannelIdFromTags from the production
 * threading module — derivation is not stubbed so the correct tag shape is
 * exercised.
 *
 * Key threading invariant (from threading.ts):
 *   getThreadReference returns { parentId: null, rootId: null } unless a
 *   "reply"-marked e-tag exists. A root-only event has no parentId/rootId.
 *   A full nested reply has ["e", rootId, "", "root"] AND
 *                           ["e", parentId, "", "reply"].
 *
 * Tests cover:
 *   - depth-2 reply (root + reply tags) → conversationId == rootId
 *   - depth-1 reply (reply tag only, no root tag) → conversationId == parentId
 *   - top-level event (no e-tags) → conversationId == event.id
 *   - malformed key (empty, whitespace, short, non-hex eventId — all reject before fetch)
 *   - resolution failure (getEventById throws)
 *   - event ID mismatch (relay returned wrong event)
 *   - channel tag mismatch
 *   - renameDraftEntry collision → null, no navigation
 *   - renameDraftEntry noop → null, no navigation
 *   - non-inbox-reply prefix → null immediately
 */

import assert from "node:assert/strict";
import test from "node:test";

import { getChannelIdFromTags, getThreadReference } from "./threading.ts";
import { migrateInboxReplyDraft } from "./inboxReplyMigration.ts";

// ── Fixtures ──────────────────────────────────────────────────────────────────

// Three distinct IDs: ROOT_ID is the thread root, PARENT_ID is the direct
// parent of the selected event, EVENT_ID is the selected inbox item itself.
const ROOT_ID = "a0".repeat(32);
const PARENT_ID = "b1".repeat(32);
const EVENT_ID = "c2".repeat(32); // distinct from PARENT_ID
const CHANNEL_ID = "chan-general";

/**
 * Depth-2 reply: root + reply e-tags. The selected event (EVENT_ID) is a
 * reply to PARENT_ID whose thread root is ROOT_ID.
 * getThreadReference returns { rootId: ROOT_ID, parentId: PARENT_ID }.
 */
function makeNestedReplyEvent({ id = EVENT_ID, channelId = CHANNEL_ID } = {}) {
  return {
    id,
    tags: [
      ["h", channelId],
      ["e", ROOT_ID, "", "root"],
      ["e", PARENT_ID, "", "reply"],
    ],
    kind: 1,
    content: "x",
    pubkey: "pk",
    created_at: 1,
  };
}

/**
 * Depth-1 reply: reply e-tag only (no root tag). The selected event (EVENT_ID)
 * is a direct reply to ROOT_ID with no deeper nesting.
 * getThreadReference returns { rootId: ROOT_ID, parentId: ROOT_ID }
 * (rootId falls back to parentId when no root tag exists, per threading.ts:50).
 */
function makeTopLevelReplyEvent({
  id = EVENT_ID,
  channelId = CHANNEL_ID,
} = {}) {
  return {
    id,
    tags: [
      ["h", channelId],
      ["e", ROOT_ID, "", "reply"],
    ],
    kind: 1,
    content: "x",
    pubkey: "pk",
    created_at: 1,
  };
}

/**
 * Top-level event: no e-tags at all.
 * getThreadReference returns { rootId: null, parentId: null }.
 * Migration falls back to event.id.
 */
function makeTopLevelEvent({ id = EVENT_ID, channelId = CHANNEL_ID } = {}) {
  return {
    id,
    tags: [["h", channelId]],
    kind: 1,
    content: "x",
    pubkey: "pk",
    created_at: 1,
  };
}

function makeDraft({ channelId = CHANNEL_ID } = {}) {
  return {
    content: "Draft text",
    selectionStart: 10,
    selectionEnd: 10,
    channelId,
    createdAt: "2025-10-01T00:00:00.000Z",
    updatedAt: "2025-10-01T00:00:00.000Z",
    pendingImeta: [],
    spoileredAttachmentUrls: [],
    status: "active",
  };
}

function makeDeps({ event, renameResult = "migrated" } = {}) {
  const renameLog = [];
  return {
    getEventById: async (id) => {
      if (!event || id !== event.id) throw new Error("not found");
      return event;
    },
    getChannelIdFromTags, // real production function
    getThreadReference, // real production function
    renameDraftEntry: (oldKey, newKey) => {
      renameLog.push({ oldKey, newKey });
      return renameResult;
    },
    renameLog,
  };
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test("depth-2 reply: conversationId is rootId from root+reply tags", async () => {
  const event = makeNestedReplyEvent();
  const deps = makeDeps({ event });

  const result = await migrateInboxReplyDraft(
    `inbox-reply:${EVENT_ID}`,
    makeDraft(),
    deps,
  );

  assert.ok(result, "expected non-null result");
  assert.equal(result.conversationId, ROOT_ID);
  assert.equal(result.newDraftKey, `thread:${ROOT_ID}`);
  assert.equal(result.channelId, CHANNEL_ID);
  assert.equal(deps.renameLog.length, 1);
  assert.equal(deps.renameLog[0].oldKey, `inbox-reply:${EVENT_ID}`);
  assert.equal(deps.renameLog[0].newKey, `thread:${ROOT_ID}`);
});

test("depth-1 reply: reply-only tag → conversationId is parentId (== rootId)", async () => {
  // When only a reply tag exists: rootId falls back to parentId (ROOT_ID here).
  const event = makeTopLevelReplyEvent();
  const deps = makeDeps({ event });

  const result = await migrateInboxReplyDraft(
    `inbox-reply:${EVENT_ID}`,
    makeDraft(),
    deps,
  );

  assert.ok(result);
  assert.equal(result.conversationId, ROOT_ID);
  assert.equal(result.newDraftKey, `thread:${ROOT_ID}`);
});

test("top-level event (no e-tags): conversationId falls back to event.id", async () => {
  const event = makeTopLevelEvent();
  const deps = makeDeps({ event });

  const result = await migrateInboxReplyDraft(
    `inbox-reply:${EVENT_ID}`,
    makeDraft(),
    deps,
  );

  assert.ok(result);
  assert.equal(result.conversationId, EVENT_ID);
  assert.equal(result.newDraftKey, `thread:${EVENT_ID}`);
});

test("malformed key — empty eventId — returns null without fetch", async () => {
  let fetched = false;
  const deps = makeDeps({ event: makeNestedReplyEvent() });
  deps.getEventById = async () => {
    fetched = true;
    return makeNestedReplyEvent();
  };

  assert.equal(
    await migrateInboxReplyDraft("inbox-reply:", makeDraft(), deps),
    null,
  );
  assert.equal(fetched, false);
  assert.equal(deps.renameLog.length, 0);
});

test("malformed key — whitespace-only eventId — returns null without fetch", async () => {
  let fetched = false;
  const deps = makeDeps({ event: makeNestedReplyEvent() });
  deps.getEventById = async () => {
    fetched = true;
    return makeNestedReplyEvent();
  };

  assert.equal(
    await migrateInboxReplyDraft("inbox-reply:   ", makeDraft(), deps),
    null,
  );
  assert.equal(fetched, false);
});

test("malformed key — short hex (63 chars) — returns null without fetch", async () => {
  let fetched = false;
  const deps = makeDeps({ event: makeNestedReplyEvent() });
  deps.getEventById = async () => {
    fetched = true;
    return makeNestedReplyEvent();
  };

  // One character short of a valid 64-char event ID.
  const shortId = "a".repeat(63);
  assert.equal(
    await migrateInboxReplyDraft(`inbox-reply:${shortId}`, makeDraft(), deps),
    null,
  );
  assert.equal(
    fetched,
    false,
    "getEventById must not be called for a short ID",
  );
  assert.equal(deps.renameLog.length, 0);
});

test("malformed key — non-hex characters in eventId — returns null without fetch", async () => {
  let fetched = false;
  const deps = makeDeps({ event: makeNestedReplyEvent() });
  deps.getEventById = async () => {
    fetched = true;
    return makeNestedReplyEvent();
  };

  // 64 chars but contains 'g' — outside [0-9a-f] entirely, rejected by /^[0-9a-f]{64}$/i.
  const nonHexId = "g".repeat(64);
  assert.equal(
    await migrateInboxReplyDraft(`inbox-reply:${nonHexId}`, makeDraft(), deps),
    null,
  );
  assert.equal(
    fetched,
    false,
    "getEventById must not be called for non-hex ID",
  );
  assert.equal(deps.renameLog.length, 0);
});

test("resolution failure (getEventById throws) returns null", async () => {
  const deps = makeDeps({ event: makeNestedReplyEvent() });
  deps.getEventById = async () => {
    throw new Error("relay timeout");
  };

  assert.equal(
    await migrateInboxReplyDraft(`inbox-reply:${EVENT_ID}`, makeDraft(), deps),
    null,
  );
  assert.equal(deps.renameLog.length, 0);
});

test("event ID mismatch returns null without rename", async () => {
  const OTHER_ID = "ff".repeat(32);
  const deps = makeDeps({ event: makeNestedReplyEvent() });
  // Relay returns an event with a different id than requested.
  deps.getEventById = async (_id) => ({
    ...makeNestedReplyEvent(),
    id: OTHER_ID,
  });

  assert.equal(
    await migrateInboxReplyDraft(`inbox-reply:${EVENT_ID}`, makeDraft(), deps),
    null,
  );
  assert.equal(
    deps.renameLog.length,
    0,
    "renameDraftEntry must not be called on id mismatch",
  );
});

test("channel tag mismatch returns null without rename", async () => {
  const event = makeNestedReplyEvent({ channelId: "chan-other" });
  const deps = makeDeps({ event });

  assert.equal(
    await migrateInboxReplyDraft(
      `inbox-reply:${EVENT_ID}`,
      makeDraft({ channelId: CHANNEL_ID }),
      deps,
    ),
    null,
  );
  assert.equal(deps.renameLog.length, 0);
});

test("renameDraftEntry collision → result null (both drafts preserved)", async () => {
  const deps = makeDeps({
    event: makeNestedReplyEvent(),
    renameResult: "collision",
  });
  const result = await migrateInboxReplyDraft(
    `inbox-reply:${EVENT_ID}`,
    makeDraft(),
    deps,
  );

  assert.equal(result, null);
  assert.equal(deps.renameLog.length, 1, "rename was attempted");
});

test("renameDraftEntry noop → result null (key vanished mid-flight)", async () => {
  const deps = makeDeps({
    event: makeNestedReplyEvent(),
    renameResult: "noop",
  });
  const result = await migrateInboxReplyDraft(
    `inbox-reply:${EVENT_ID}`,
    makeDraft(),
    deps,
  );

  assert.equal(result, null);
  assert.equal(deps.renameLog.length, 1);
});

test("non-inbox-reply prefix returns null immediately without any I/O", async () => {
  let fetched = false;
  const deps = makeDeps({ event: makeNestedReplyEvent() });
  deps.getEventById = async () => {
    fetched = true;
    return makeNestedReplyEvent();
  };

  assert.equal(
    await migrateInboxReplyDraft(`thread:${ROOT_ID}`, makeDraft(), deps),
    null,
  );
  assert.equal(fetched, false);
  assert.equal(deps.renameLog.length, 0);
});
