import assert from "node:assert/strict";
import test from "node:test";

import { MENTION_TAG_CAP, replyRecipientPubkeys } from "./threading.ts";

const ME = "a".repeat(64);
const PARENT = "b".repeat(64);
const body = (n) =>
  Array.from({ length: n }, (_, i) => `${i}`.padStart(64, "c"));

test("appends the parent author and drops self", () => {
  assert.deepEqual(
    replyRecipientPubkeys({
      currentPubkey: ME,
      mentionPubkeys: [ME, PARENT.toUpperCase()],
      parentAuthorPubkey: PARENT,
    }),
    [PARENT],
  );
});

test("a full mention list leaves room for the addressing tag", () => {
  // The addressing tag is no longer in this list — the backend appends it,
  // marked, from the parent event it already fetched. What this function still
  // owes is the slot: the builder rejects the whole event past the cap rather
  // than trimming, so a full body list plus the backend's tag is a failed send,
  // and losing the addressing tag would cost the agent the reply entirely.
  const result = replyRecipientPubkeys({
    currentPubkey: ME,
    mentionPubkeys: body(MENTION_TAG_CAP),
    parentAuthorPubkey: PARENT,
  });
  assert.equal(result.length, MENTION_TAG_CAP - 1);
  assert.ok(!result.includes(PARENT));
});

test("an under-cap list is left alone", () => {
  const result = replyRecipientPubkeys({
    currentPubkey: ME,
    mentionPubkeys: body(3),
    parentAuthorPubkey: PARENT,
  });
  assert.equal(result.length, 3);
  assert.ok(!result.includes(PARENT));
});

test("a parent already mentioned in the body is not duplicated", () => {
  const result = replyRecipientPubkeys({
    currentPubkey: ME,
    mentionPubkeys: [PARENT, ...body(2)],
    parentAuthorPubkey: PARENT,
  });
  assert.deepEqual(
    result.filter((pk) => pk === PARENT),
    [PARENT],
  );
});

test("no parent author leaves the list untouched", () => {
  assert.deepEqual(
    replyRecipientPubkeys({
      currentPubkey: ME,
      mentionPubkeys: body(2),
      parentAuthorPubkey: null,
    }),
    body(2),
  );
});
