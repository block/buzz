import assert from "node:assert/strict";
import test from "node:test";

import {
  buildVideoReviewCommentsByRootId,
  buildVideoReviewCommentsForRoot,
  buildVideoReviewContextForMessage,
  hasVideoAttachment,
} from "./videoReviewContext.ts";

function message(overrides) {
  return {
    id: "message",
    createdAt: 1,
    pubkey: "author",
    author: "Author",
    avatarUrl: null,
    role: undefined,
    personaDisplayName: undefined,
    time: "12:00 PM",
    body: "body",
    parentId: null,
    rootId: null,
    depth: 0,
    accent: false,
    pending: undefined,
    edited: false,
    kind: 9,
    tags: [],
    reactions: undefined,
    ...overrides,
  };
}

test("hasVideoAttachment detects markdown and imeta videos", () => {
  assert.equal(
    hasVideoAttachment(
      message({ body: "Launch cut\n![video](https://relay/media/a.mp4)" }),
    ),
    true,
  );

  assert.equal(
    hasVideoAttachment(
      message({
        tags: [
          [
            "imeta",
            "url https://relay/media/a.mp4",
            "m video/mp4",
            "dim 1920x1080",
          ],
        ],
      }),
    ),
    true,
  );

  assert.equal(hasVideoAttachment(message({ body: "plain text" })), false);
});

test("buildVideoReviewCommentsByRootId includes nested descendants", () => {
  const video = message({
    id: "video",
    body: "![video](https://relay/media/a.mp4)",
    createdAt: 1,
  });
  const firstComment = message({
    id: "first-comment",
    body: "[00:01] tighten this",
    createdAt: 3,
    parentId: "video",
    rootId: "video",
  });
  const nestedReply = message({
    id: "nested-reply",
    body: "agreed",
    createdAt: 4,
    parentId: "first-comment",
    rootId: "video",
  });
  const earlierComment = message({
    id: "earlier-comment",
    body: "[00:00] opener",
    createdAt: 2,
    parentId: "video",
    rootId: "video",
  });

  const commentsByRootId = buildVideoReviewCommentsByRootId([
    video,
    firstComment,
    nestedReply,
    earlierComment,
  ]);

  assert.deepEqual(
    commentsByRootId.get("video")?.map((comment) => comment.id),
    ["earlier-comment", "first-comment", "nested-reply"],
  );
});

test("buildVideoReviewCommentsForRoot returns descendants for one root", () => {
  const video = message({
    id: "video",
    body: "![video](https://relay/media/a.mp4)",
    createdAt: 1,
  });
  const otherVideo = message({
    id: "other-video",
    body: "![video](https://relay/media/b.mp4)",
    createdAt: 2,
  });
  const firstComment = message({
    id: "first-comment",
    body: "[00:01] tighten this",
    createdAt: 4,
    parentId: "video",
    rootId: "video",
  });
  const nestedReply = message({
    id: "nested-reply",
    body: "agreed",
    createdAt: 5,
    parentId: "first-comment",
    rootId: "video",
  });
  const earlierComment = message({
    id: "earlier-comment",
    body: "[00:00] opener",
    createdAt: 3,
    parentId: "video",
    rootId: "video",
  });
  const otherComment = message({
    id: "other-comment",
    body: "different root",
    createdAt: 6,
    parentId: "other-video",
    rootId: "other-video",
  });

  const comments = buildVideoReviewCommentsForRoot(
    [
      video,
      otherVideo,
      firstComment,
      nestedReply,
      earlierComment,
      otherComment,
    ],
    "video",
  );

  assert.deepEqual(
    comments.map((comment) => comment.id),
    ["earlier-comment", "first-comment", "nested-reply"],
  );
});

test("buildVideoReviewContextForMessage posts against the source video", async () => {
  const video = message({
    id: "video",
    body: "![video](https://relay/media/a.mp4)",
    createdAt: 1,
  });
  const comment = message({
    id: "comment",
    body: "[00:01] tighten this",
    createdAt: 2,
    parentId: "video",
    rootId: "video",
  });
  const calls = [];

  const context = buildVideoReviewContextForMessage({
    channelId: "channel",
    comments: [comment],
    message: video,
    onSendVideoReviewComment: async (
      source,
      content,
      mentionPubkeys,
      mediaTags,
      parentEventId,
    ) => {
      calls.push({
        content,
        mediaTags,
        mentionPubkeys,
        parentEventId,
        sourceId: source.id,
      });
    },
  });

  assert.equal(context?.rootEventId, "video");
  assert.equal(context?.comments[0].id, "comment");

  await context?.onSendComment?.("looks good", ["alice"], undefined, "comment");

  assert.deepEqual(calls, [
    {
      content: "looks good",
      mediaTags: undefined,
      mentionPubkeys: ["alice"],
      parentEventId: "comment",
      sourceId: "video",
    },
  ]);
});
