import assert from "node:assert/strict";
import { test } from "node:test";

import {
  commitDiscussionQuery,
  discussionSnippet,
  entityDiscussionQuery,
  formatNameList,
  groupDiscussionChannels,
  repositoryDiscussionQuery,
} from "./discussionChannels.ts";

const OWNER = "a".repeat(64);
const EVENT_ID = "b".repeat(64);
const ALICE = "c".repeat(64);
const BOB = "d".repeat(64);

test("query builders emit the tokens FTS needs, nothing else", () => {
  assert.equal(entityDiscussionQuery(EVENT_ID), EVENT_ID);
  assert.equal(
    repositoryDiscussionQuery({ owner: OWNER, dtag: "buzz-world" }),
    `${OWNER} buzz-world`,
  );
});

test("commit queries match full or short hash citations", () => {
  const hash = "0123456789abcdef0123456789abcdef01234567";
  assert.equal(
    commitDiscussionQuery({ hash, shortHash: "0123456" }),
    `${hash} OR 0123456`,
  );
  // Short hash derives from the full hash when the snapshot omitted it.
  assert.equal(commitDiscussionQuery({ hash }), `${hash} OR 0123456`);
  // Degenerate case: already-short hashes search as a single token.
  assert.equal(commitDiscussionQuery({ hash: "0123456" }), "0123456");
});

test("hits group into channels ordered by count then recency", () => {
  const channels = groupDiscussionChannels([
    { channelId: "c1", channelName: "general", createdAt: 100, pubkey: ALICE },
    { channelId: "c2", channelName: "design", createdAt: 300, pubkey: BOB },
    { channelId: "c1", channelName: "general", createdAt: 200, pubkey: BOB },
    { channelId: "c3", channelName: "random", createdAt: 300, pubkey: ALICE },
  ]);
  assert.deepEqual(channels, [
    {
      id: "c1",
      name: "general",
      messageCount: 2,
      lastActivityAt: 200,
      // Most recent speaker first.
      participants: [BOB, ALICE],
    },
    // c2 and c3 tie on count; newer activity first (stable tie broken by time).
    {
      id: "c2",
      name: "design",
      messageCount: 1,
      lastActivityAt: 300,
      participants: [BOB],
    },
    {
      id: "c3",
      name: "random",
      messageCount: 1,
      lastActivityAt: 300,
      participants: [ALICE],
    },
  ]);
});

test("channel-less hits are dropped, names backfill, participants dedupe", () => {
  const channels = groupDiscussionChannels([
    { channelId: null, channelName: null, createdAt: 100, pubkey: ALICE },
    { channelId: "c1", channelName: null, createdAt: 100, pubkey: ALICE },
    {
      channelId: "c1",
      channelName: "general",
      createdAt: 50,
      pubkey: ALICE.toUpperCase(),
    },
  ]);
  assert.deepEqual(channels, [
    {
      id: "c1",
      name: "general",
      messageCount: 2,
      lastActivityAt: 100,
      participants: [ALICE],
    },
  ]);
});

test("formatNameList reads naturally at every size", () => {
  assert.equal(formatNameList([]), "");
  assert.equal(formatNameList(["Alice"]), "Alice");
  assert.equal(formatNameList(["Alice", "Bob"]), "Alice and Bob");
  assert.equal(
    formatNameList(["Alice", "Bob", "Carol"]),
    "Alice, Bob and Carol",
  );
  assert.equal(
    formatNameList(["Alice", "Bob", "Carol", "Dan"]),
    "Alice, Bob and 2 others",
  );
});

test("discussionSnippet strips entity links and coordinates", () => {
  assert.equal(
    discussionSnippet(
      `Can someone review buzz://pr?id=${EVENT_ID}&owner=${OWNER}&d=buzz before Friday?`,
    ),
    "Can someone review before Friday?",
  );
  assert.equal(
    discussionSnippet(`Deploying 30617:${OWNER}:relay-tools tonight`),
    "Deploying tonight",
  );
  assert.equal(
    discussionSnippet(`buzz://repo?owner=${OWNER}&d=buzz`),
    "Shared a link to this.",
  );
});

test("discussionSnippet truncates long content on an ellipsis", () => {
  // The cap only bounds DOM size — the row's CSS truncation does the
  // visual cut — so it just needs to hold for arbitrarily long content.
  const long = "word ".repeat(200);
  const snippet = discussionSnippet(long);
  assert.ok(snippet.length <= 400);
  assert.ok(snippet.endsWith("…"));
});
