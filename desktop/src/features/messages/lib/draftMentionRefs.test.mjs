import assert from "node:assert/strict";
import test from "node:test";

import {
  buildEditMentionState,
  buildMessageComposerEditTarget,
  resolveEditMentionRefs,
} from "./draftMentionRefs.ts";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);
const message = (body, tags) => ({
  author: "Alice",
  body,
  id: "message-id",
  tags,
});

const profiles = {
  [ALICE]: { displayName: "Alice" },
  [BOB]: { displayName: "Bob" },
};

test("edit mention refs resolve from visible text and loaded profiles", () => {
  assert.deepEqual(
    resolveEditMentionRefs(
      "Please review this, @Alice.",
      [["p", ALICE]],
      profiles,
      () => false,
    ),
    [{ displayName: "Alice", isAgent: false, pubkey: ALICE }],
  );

  const target = buildMessageComposerEditTarget(
    message("Please review this, @Alice.", [["p", ALICE]]),
    profiles,
    () => false,
  );
  assert.deepEqual(target.unresolvedMentionPubkeys, []);
});

test("shared edit mention state preserves tagged identities while profiles are unavailable", () => {
  assert.deepEqual(
    buildEditMentionState(
      "Please review this, @Alice and @Bob.",
      [
        ["p", ALICE],
        ["mention", BOB],
      ],
      undefined,
      () => false,
    ),
    { mentionRefs: [], unresolvedMentionPubkeys: [ALICE, BOB] },
  );
});

test("edit target preserves tagged identities while profiles are unavailable", () => {
  const target = buildMessageComposerEditTarget(
    message("Please review this, @Alice and @Bob.", [
      ["p", ALICE],
      ["mention", BOB],
    ]),
    undefined,
    () => false,
  );

  assert.deepEqual(target.mentionRefs, []);
  assert.deepEqual(target.unresolvedMentionPubkeys, [ALICE, BOB]);
});

test("edit target separates resolved refs from identities missing profiles", () => {
  const target = buildMessageComposerEditTarget(
    message("Please review this, @Alice and @Bob.", [
      ["p", ALICE],
      ["mention", BOB],
    ]),
    { [ALICE]: profiles[ALICE] },
    () => false,
  );

  assert.deepEqual(target.mentionRefs, [
    { displayName: "Alice", isAgent: false, pubkey: ALICE },
  ]);
  assert.deepEqual(target.unresolvedMentionPubkeys, [BOB]);
});
