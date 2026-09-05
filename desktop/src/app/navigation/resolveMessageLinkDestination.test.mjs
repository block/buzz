/**
 * Regression for #7315: `buzz://message` links into a forum channel routed
 * through `goChannel`, and `/channels/$channelId` cannot select a forum post
 * (it hardcodes `selectedPostId={null}`), so the link landed on the post list
 * with nothing selected. Search already resolved this correctly via
 * `resolveSearchHitDestination`; this mirrors that branching for message links.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { resolveMessageLinkDestination } from "./resolveMessageLinkDestination.ts";

const CHANNEL = "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11";
const KIND_STREAM_MESSAGE = 9;
const KIND_FORUM_POST = 45001;
const KIND_FORUM_COMMENT = 45003;

function fetcher(event) {
  return async () => event;
}

test("a forum post link opens the post, not the post list", async () => {
  const destination = await resolveMessageLinkDestination(
    CHANNEL,
    "post-1",
    null,
    fetcher({ id: "post-1", kind: KIND_FORUM_POST, tags: [] }),
  );

  assert.deepEqual(destination, {
    kind: "forum-post",
    channelId: CHANNEL,
    postId: "post-1",
  });
});

test("a forum comment link opens its post and carries the reply id", async () => {
  const destination = await resolveMessageLinkDestination(
    CHANNEL,
    "comment-1",
    null,
    fetcher({
      id: "comment-1",
      kind: KIND_FORUM_COMMENT,
      // A top-level forum comment carries both markers, the way
      // `getThreadReference` expects: a `root` pointing at the post and a
      // `reply` pointing at whatever it answers.
      tags: [
        ["e", "post-1", "", "root"],
        ["e", "post-1", "", "reply"],
      ],
    }),
  );

  assert.deepEqual(destination, {
    kind: "forum-post",
    channelId: CHANNEL,
    postId: "post-1",
    replyId: "comment-1",
  });
});

test("a stream message link keeps the channel destination", async () => {
  const destination = await resolveMessageLinkDestination(
    CHANNEL,
    "message-1",
    "root-1",
    fetcher({ id: "message-1", kind: KIND_STREAM_MESSAGE, tags: [] }),
  );

  assert.deepEqual(destination, {
    kind: "channel",
    channelId: CHANNEL,
    messageId: "message-1",
    threadRootId: "root-1",
  });
});

test("a comment with no resolvable root falls back to the channel", async () => {
  const destination = await resolveMessageLinkDestination(
    CHANNEL,
    "comment-2",
    null,
    fetcher({ id: "comment-2", kind: KIND_FORUM_COMMENT, tags: [] }),
  );

  assert.equal(destination.kind, "channel");
  assert.equal(destination.channelId, CHANNEL);
});

test("a failed lookup never makes the link unclickable", async () => {
  const destination = await resolveMessageLinkDestination(
    CHANNEL,
    "message-3",
    null,
    async () => {
      throw new Error("relay unavailable");
    },
  );

  assert.deepEqual(destination, {
    kind: "channel",
    channelId: CHANNEL,
    messageId: "message-3",
    threadRootId: null,
  });
});
