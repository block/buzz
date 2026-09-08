import assert from "node:assert/strict";
import test from "node:test";

import {
  buildVideoReviewCommentsByRootId,
  buildVideoReviewCommentsForRoot,
  buildVideoReviewCommentRootIdsByMessageId,
  buildVideoReviewContextForMessage,
  buildVideoReviewContextsByMessageId,
  hasRenderedVideoAttachment,
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
  assert.equal(
    hasVideoAttachment(
      message({
        body: "orphan metadata only",
        tags: [["imeta", "url https://cdn.example.com/cut.mp4", "m video/mp4"]],
      }),
    ),
    true,
  );
  assert.equal(
    hasRenderedVideoAttachment(
      message({
        body: "orphan metadata only",
        tags: [["imeta", "url https://cdn.example.com/cut.mp4", "m video/mp4"]],
      }),
    ),
    false,
  );
});
test("hasVideoAttachment uses the Markdown renderer's video classification", () => {
  assert.equal(
    hasVideoAttachment(
      message({ body: "![Demo](https://cdn.example.com/cut.mp4)" }),
    ),
    true,
  );
  assert.equal(
    hasVideoAttachment(
      message({ body: "![Poster](https://cdn.example.com/cut.jpg)" }),
    ),
    false,
  );
  assert.equal(
    hasVideoAttachment(
      message({
        body: "![Demo](https://relay/media/cut.mp4)",
        tags: [["imeta", "url https://relay/media/cut.mp4", "m image/png"]],
      }),
    ),
    false,
  );
  assert.equal(
    hasVideoAttachment(
      message({
        body: "![Demo][clip]\n\n[clip]: https://cdn.example.com/cut.mp4",
      }),
    ),
    true,
  );
  assert.equal(
    hasVideoAttachment(
      message({
        body: "```md\n![Demo](https://cdn.example.com/cut.mp4)\n```",
      }),
    ),
    false,
  );
});

test("packaged MP4 voice notes are not video-review roots", () => {
  const voiceNote = message({
    body: "[voice-note-123.mp4](https://relay/media/voice.mp4)",
    tags: [
      [
        "imeta",
        "url https://relay/media/voice.mp4",
        "m video/mp4",
        "filename voice-note-123.mp4",
      ],
    ],
  });

  assert.equal(hasVideoAttachment(voiceNote), false);
  assert.equal(hasRenderedVideoAttachment(voiceNote), false);
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

test("buildVideoReviewCommentRootIdsByMessageId targets the nearest video ancestor", () => {
  const root = message({ id: "root", body: "Review request" });
  const firstVideo = message({
    id: "first-video",
    body: "![video](https://relay/media/a.mp4)",
    parentId: root.id,
    rootId: root.id,
  });
  const firstComment = message({
    id: "first-comment",
    body: "[00:01] tighten this",
    parentId: firstVideo.id,
    rootId: root.id,
  });
  const nestedVideo = message({
    id: "nested-video",
    body: "![video](https://relay/media/b.mp4)",
    parentId: firstComment.id,
    rootId: root.id,
  });
  const nestedComment = message({
    id: "nested-comment",
    body: "[00:02] check this frame",
    parentId: nestedVideo.id,
    rootId: root.id,
  });
  const plainReply = message({
    id: "plain-reply",
    body: "No video ancestor",
    parentId: root.id,
    rootId: root.id,
  });

  const rootIds = buildVideoReviewCommentRootIdsByMessageId([
    root,
    firstVideo,
    firstComment,
    nestedVideo,
    nestedComment,
    plainReply,
  ]);

  assert.deepEqual(
    [...rootIds.entries()],
    [
      [firstComment.id, firstVideo.id],
      [nestedComment.id, nestedVideo.id],
    ],
  );
});

test("buildVideoReviewCommentRootIdsByMessageId can require rendered video roots", () => {
  const orphanVideo = message({
    id: "orphan-video",
    body: "metadata only",
    tags: [["imeta", "url https://relay/media/a.mp4", "m video/mp4"]],
  });
  const comment = message({
    id: "comment",
    body: "[00:01] review this",
    parentId: orphanVideo.id,
    rootId: orphanVideo.id,
  });

  assert.deepEqual(
    [
      ...buildVideoReviewCommentRootIdsByMessageId([
        orphanVideo,
        comment,
      ]).entries(),
    ],
    [[comment.id, orphanVideo.id]],
  );
  assert.deepEqual(
    [
      ...buildVideoReviewCommentRootIdsByMessageId(
        [orphanVideo, comment],
        hasRenderedVideoAttachment,
      ).entries(),
    ],
    [],
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

test("buildVideoReviewContextsByMessageId includes video replies", () => {
  const root = message({ id: "root", body: "Review request" });
  const videoReply = message({
    id: "video-reply",
    body: "![video](https://relay/media/a.mp4)",
    parentId: root.id,
    rootId: root.id,
  });
  const comment = message({
    id: "comment",
    body: "[00:01] tighten this",
    parentId: videoReply.id,
    rootId: root.id,
  });

  const contexts = buildVideoReviewContextsByMessageId({
    channelId: "channel",
    messages: [root, videoReply, comment],
  });

  assert.deepEqual([...contexts.keys()], [videoReply.id]);
  assert.deepEqual(
    contexts.get(videoReply.id)?.comments.map((item) => item.id),
    [comment.id],
  );
});
