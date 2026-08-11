import assert from "node:assert/strict";
import test from "node:test";

import { submitMessageEdit } from "./submitMessageEdit.ts";

const UNRESOLVED_USER = "b".repeat(64);

function baseOptions(
  save,
  {
    content = "hello @Missing User",
    editTarget = {
      mentionRefs: [],
      unresolvedMentionPubkeys: [UNRESOLVED_USER],
    },
  } = {},
) {
  return {
    clearComposer: () => {},
    content,
    customEmoji: [],
    editTarget,
    editTargetId: "event-id",
    extractMentionPubkeys: () => [],
    getMentionRefs: () => [],
    originalContent: content,
    ownerPubkey: "a".repeat(64),
    pendingImeta: [],
    queuedAttachments: [],
    restoreComposer: () => {},
    restoreMentionRefs: () => {},
    setDeferredUploadPending: () => {},
    setUploadError: () => {},
    shouldRestoreComposer: () => true,
    spoileredAttachmentUrls: new Set(),
    save,
  };
}

test("edit save emits unresolved identities as non-notifying mention references", async () => {
  let saved;
  await submitMessageEdit(
    baseOptions(async (content, tags, mentionPubkeys, eventId) => {
      saved = { content, tags, mentionPubkeys, eventId };
    }),
  );

  assert.deepEqual(saved, {
    content: "hello @Missing User",
    tags: [["mention", UNRESOLVED_USER]],
    mentionPubkeys: [],
    eventId: "event-id",
  });
});

test("edit save uses edit-target refs that resolve after edit-open", async () => {
  let saved;
  const resolvedRef = {
    displayName: "Missing User",
    isAgent: false,
    pubkey: UNRESOLVED_USER,
  };
  await submitMessageEdit(
    baseOptions(
      async (content, tags, mentionPubkeys, eventId) => {
        saved = { content, tags, mentionPubkeys, eventId };
      },
      {
        editTarget: {
          mentionRefs: [resolvedRef],
          unresolvedMentionPubkeys: [],
        },
      },
    ),
  );

  assert.deepEqual(saved, {
    content: "hello @Missing User",
    tags: [["mention", UNRESOLVED_USER]],
    mentionPubkeys: [],
    eventId: "event-id",
  });
});
