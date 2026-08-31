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
    revalidateMentionPubkeys: async (pubkeys) => [...pubkeys],
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

test("edit save revalidates added mentions immediately before save", async () => {
  const agent = "c".repeat(64);
  const calls = [];
  await submitMessageEdit({
    ...baseOptions(async (_content, _tags, mentionPubkeys) => {
      calls.push(["save", mentionPubkeys]);
    }),
    content: "hello @Agent",
    originalContent: "hello",
    extractMentionPubkeys: (content) =>
      content.includes("@Agent") ? [agent] : [],
    revalidateMentionPubkeys: async (pubkeys) => {
      calls.push(["revalidate", pubkeys]);
      return [];
    },
  });

  assert.deepEqual(calls, [
    ["revalidate", [agent]],
    ["save", []],
  ]);
});

test("edit upload pause revalidates revoked mentions only after upload completes", async () => {
  const agent = "d".repeat(64);
  const calls = [];
  let completeUpload;
  await submitMessageEdit({
    ...baseOptions(async (_content, _tags, mentionPubkeys) => {
      calls.push(["save", mentionPubkeys]);
    }),
    content: "hello @Agent",
    originalContent: "hello",
    extractMentionPubkeys: (content) =>
      content.includes("@Agent") ? [agent] : [],
    queuedAttachments: [
      {
        file: new File(["image"], "image.png", { type: "image/png" }),
        id: 1,
        spoilered: false,
      },
    ],
    enqueueUpload: ({ onComplete }) => {
      completeUpload = () => onComplete([], new AbortController().signal);
      return {};
    },
    revalidateMentionPubkeys: async (pubkeys) => {
      calls.push(["revalidate", pubkeys]);
      return [];
    },
  });

  assert.deepEqual(calls, []);
  await completeUpload();
  assert.deepEqual(calls, [
    ["revalidate", [agent]],
    ["save", []],
  ]);
});

test("ambiguous extractor failure is visible before edit draft clearing or save", async () => {
  const calls = [];
  const error =
    "The mention @Scout is ambiguous. Choose a recipient from the mention picker.";
  await submitMessageEdit({
    ...baseOptions(async () => calls.push("save")),
    extractMentionPubkeys: () => {
      throw new Error(error);
    },
    clearComposer: () => calls.push("clear"),
    setUploadError: (message) => calls.push(message),
  });
  assert.deepEqual(calls, [error]);
});

for (const replacement of ["hello", "hello @Alice"]) {
  test(`an ambiguous historical mention can be replaced with ${replacement}`, async () => {
    const { extractMentionPubkeys } = await import(
      "../lib/extractMentionPubkeys.ts"
    );
    const alice = "e".repeat(64);
    const calls = [];
    await submitMessageEdit({
      ...baseOptions(async (_content, _tags, pubkeys) =>
        calls.push(["save", pubkeys]),
      ),
      content: replacement,
      originalContent: "hello @Scout",
      extractMentionPubkeys: (text) =>
        extractMentionPubkeys({
          text,
          selectedMentions: new Map(),
          memberCandidates: [
            { displayName: "Scout", pubkey: "c".repeat(64), isMember: true },
            { displayName: "Scout", pubkey: "d".repeat(64), isMember: true },
            { displayName: "Alice", pubkey: alice, isMember: true },
          ],
        }),
      revalidateMentionPubkeys: async (pubkeys) => {
        calls.push(["revalidate", pubkeys]);
        return pubkeys;
      },
      setUploadError: (error) => calls.push(["error", error]),
    });
    const expected = replacement.includes("@Alice") ? [alice] : [];
    assert.deepEqual(calls, [
      ["revalidate", expected],
      ["save", expected],
    ]);
  });
}
