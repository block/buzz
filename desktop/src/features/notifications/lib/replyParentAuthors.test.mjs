import assert from "node:assert/strict";
import test from "node:test";

import { collectReplyParentAuthors } from "./replyParentAuthors.ts";

const ME = "a".repeat(64);
const OTHER = "b".repeat(64);

// Real event ids: an `e` value that cannot identify a relay event is skipped
// before it reaches an `ids` filter, so short labels would no longer be fetched.
const M1 = "1".repeat(64);
const M2 = "2".repeat(64);
const M3 = "3".repeat(64);
const OLDER_PARENT = "e".repeat(64);

function makeEvent(id, pubkey, tags = []) {
  return {
    id,
    pubkey,
    created_at: 1700000000,
    kind: 9,
    tags,
    content: "hi",
    sig: "s".repeat(128),
  };
}

const replyTo = (parentId, rootId = parentId) =>
  rootId === parentId
    ? [["e", parentId, "", "reply"]]
    : [
        ["e", rootId, "", "root"],
        ["e", parentId, "", "reply"],
      ];

const neverFetch = () => {
  throw new Error("should not fetch");
};

test("maps every event in the batch by id without fetching", async () => {
  const events = [makeEvent(M1, ME), makeEvent(M2, OTHER, replyTo(M1))];
  const authors = await collectReplyParentAuthors({
    events,
    fetchEvents: neverFetch,
    kinds: [9],
    shouldResolveParent: () => true,
  });
  assert.equal(authors.get(M1), ME);
  assert.equal(authors.get(M2), OTHER);
});

test("skips the fetch when shouldResolveParent returns false", async () => {
  // A real parent id, absent from the batch: with the gate open this is exactly
  // the case that fetches. `neverFetch` is the assertion — a label that
  // `normalizeEventId` rejects would skip the fetch on its own and prove
  // nothing about the gate.
  const events = [makeEvent(M2, OTHER, replyTo(OLDER_PARENT))];
  const authors = await collectReplyParentAuthors({
    events,
    fetchEvents: neverFetch,
    kinds: [9],
    shouldResolveParent: () => false,
  });
  assert.equal(authors.has(OLDER_PARENT), false);
});

test("fetches parents missing from the batch when the gate is open", async () => {
  const events = [
    makeEvent(M2, OTHER, replyTo(OLDER_PARENT)),
    makeEvent(M3, OTHER, replyTo(OLDER_PARENT)),
  ];
  const requested = [];
  const authors = await collectReplyParentAuthors({
    events,
    fetchEvents: async (filter) => {
      requested.push(filter);
      return [makeEvent(OLDER_PARENT, ME)];
    },
    kinds: [9, 40002],
    shouldResolveParent: () => true,
  });
  // One request, deduped, with the kinds the p-gate requires.
  assert.equal(requested.length, 1);
  assert.deepEqual(requested[0].ids, [OLDER_PARENT]);
  assert.deepEqual(requested[0].kinds, [9, 40002]);
  assert.equal(authors.get(OLDER_PARENT), ME);
});

test("a failed parent fetch propagates so the caller can retry", async () => {
  // Degrading to a batch-only map looks graceful but is not: the unresolved
  // parent is guessed at and the guess is persisted per event id, never
  // recomputed. Both callers treat a throw as "retry this channel".
  const events = [makeEvent(M2, OTHER, replyTo(OLDER_PARENT))];
  await assert.rejects(
    collectReplyParentAuthors({
      events,
      fetchEvents: async () => {
        throw new Error("relay down");
      },
      kinds: [9],
      shouldResolveParent: () => true,
    }),
    /relay down/,
  );
});

test("top-level events never trigger a fetch", async () => {
  const authors = await collectReplyParentAuthors({
    events: [makeEvent(M1, OTHER)],
    fetchEvents: neverFetch,
    kinds: [9],
    shouldResolveParent: () => true,
  });
  assert.equal(authors.size, 1);
});
