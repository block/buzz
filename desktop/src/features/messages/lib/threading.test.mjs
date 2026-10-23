import assert from "node:assert/strict";
import test from "node:test";

import { buildReplyTags, resolveReplyTargetAuthorPubkey } from "./threading.ts";

const AGENT_PK = "aa".repeat(32);
const SELF_PK = "bb".repeat(32);
const OTHER_PK = "cc".repeat(32);
const PARENT_ID = "11".repeat(32);
const ROOT_ID = "22".repeat(32);
const CHANNEL_ID = "00000000-0000-0000-0000-000000000000";

test("buildReplyTags includes the replied-to author as a p-tag by default", () => {
  const tags = buildReplyTags(
    CHANNEL_ID,
    SELF_PK,
    PARENT_ID,
    ROOT_ID,
    [],
    AGENT_PK,
  );
  const pTags = tags.filter((t) => t[0] === "p");
  // author + replied-to author
  assert.deepEqual(
    pTags.map((t) => t[1].toLowerCase()).sort(),
    [AGENT_PK, SELF_PK].sort(),
  );
});

test("buildReplyTags folds replied-to author together with explicit mentions and dedups", () => {
  const tags = buildReplyTags(
    CHANNEL_ID,
    SELF_PK,
    PARENT_ID,
    ROOT_ID,
    [AGENT_PK, OTHER_PK],
    AGENT_PK,
  );
  const pTags = tags.filter((t) => t[0] === "p").map((t) => t[1].toLowerCase());
  // SELF_PK (author), AGENT_PK (mention + replied-to, deduped), OTHER_PK
  assert.equal(pTags.includes(SELF_PK), true);
  assert.equal(pTags.filter((p) => p === AGENT_PK).length, 1);
  assert.equal(pTags.includes(OTHER_PK), true);
});

test("buildReplyTags does not re-mention the composer (self) as reply target", () => {
  const tags = buildReplyTags(
    CHANNEL_ID,
    SELF_PK,
    PARENT_ID,
    ROOT_ID,
    [],
    SELF_PK, // replying to own message must not add a self p-tag twice
  );
  const selfMentions = tags.filter(
    (t) => t[0] === "p" && t[1].toLowerCase() === SELF_PK,
  );
  // Only the author p-tag, no duplicate self mention.
  assert.equal(selfMentions.length, 1);
});

test("buildReplyTags omits replied-to author when not provided", () => {
  const tags = buildReplyTags(CHANNEL_ID, SELF_PK, PARENT_ID, ROOT_ID, []);
  const pTags = tags.filter((t) => t[0] === "p").map((t) => t[1].toLowerCase());
  assert.deepEqual(pTags, [SELF_PK]);
});

test("resolveReplyTargetAuthorPubkey returns the parent author from the cache", () => {
  const events = [
    { id: PARENT_ID, pubkey: AGENT_PK, tags: [] },
    { id: ROOT_ID, pubkey: OTHER_PK, tags: [] },
  ];
  assert.equal(resolveReplyTargetAuthorPubkey(PARENT_ID, events), AGENT_PK);
});

test("resolveReplyTargetAuthorPubkey returns null when parent is not in cache", () => {
  assert.equal(resolveReplyTargetAuthorPubkey(PARENT_ID, []), null);
});
