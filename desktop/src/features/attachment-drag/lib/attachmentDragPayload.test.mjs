import assert from "node:assert/strict";
import test from "node:test";

import {
  attachmentDragContentAndTags,
  attachmentDragItemsFromTags,
  attachmentDragPreview,
  parseAttachmentDragPayload,
  serializeAttachmentDragPayload,
} from "./attachmentDragPayload.ts";

test("attachment drag payload round-trips valid imeta items", () => {
  const payload = {
    type: "buzz-attachment-drag",
    version: 1,
    sourceChannelId: "channel-a",
    sourceMessageId: "message-a",
    sourceThreadRootId: "root-a",
    sourceAuthor: "Ada",
    items: [
      {
        tag: [
          "imeta",
          "url https://relay/media/a.png",
          "m image/png",
          "x abc",
          "size 12",
          "filename a.png",
        ],
        url: "https://relay/media/a.png",
        type: "image/png",
        sha256: "abc",
        size: 12,
        uploaded: 0,
        filename: "a.png",
      },
    ],
  };

  assert.deepEqual(
    parseAttachmentDragPayload(serializeAttachmentDragPayload(payload)),
    payload,
  );
});

test("attachment drag payload rejects invalid or empty payloads", () => {
  assert.equal(parseAttachmentDragPayload(null), null);
  assert.equal(parseAttachmentDragPayload("not json"), null);
  assert.equal(
    parseAttachmentDragPayload(
      JSON.stringify({
        type: "buzz-attachment-drag",
        version: 1,
        sourceMessageId: "message-a",
        items: [],
      }),
    ),
    null,
  );
  assert.equal(
    parseAttachmentDragPayload(
      JSON.stringify({
        type: "buzz-attachment-drag",
        version: 1,
        sourceMessageId: "message-a",
        items: [
          {
            tag: ["p", "bad"],
            url: "x",
            type: "image/png",
            sha256: "",
            size: 1,
            uploaded: 0,
          },
        ],
      }),
    ),
    null,
  );
});

test("attachment drag preview uses filename when available", () => {
  assert.deepEqual(
    attachmentDragPreview({
      items: [
        {
          tag: ["imeta", "url https://relay/media/a.pdf"],
          url: "https://relay/media/a.pdf",
          type: "application/pdf",
          sha256: "abc",
          size: 12,
          uploaded: 0,
          filename: "a.pdf",
        },
      ],
    }),
    { count: 1, label: "a.pdf" },
  );

  assert.deepEqual(
    attachmentDragPreview({
      items: [
        {
          tag: ["imeta"],
          url: "u1",
          type: "image/png",
          sha256: "a",
          size: 1,
          uploaded: 0,
        },
        {
          tag: ["imeta"],
          url: "u2",
          type: "image/png",
          sha256: "b",
          size: 1,
          uploaded: 0,
        },
      ],
    }),
    { count: 2, label: "2 attachments" },
  );
});

test("attachment drag items preserve source imeta tag by url", () => {
  const tags = [
    [
      "imeta",
      "url https://relay/media/a.png",
      "m image/png",
      "x abc",
      "size 12",
      "filename a.png",
    ],
    ["p", "someone"],
  ];

  assert.deepEqual(attachmentDragItemsFromTags(tags), [
    {
      tag: tags[0],
      url: "https://relay/media/a.png",
      type: "image/png",
      sha256: "abc",
      size: 12,
      uploaded: 0,
      filename: "a.png",
    },
  ]);
});

test("attachment drag content rebuilds markdown and exact imeta tags", () => {
  const tag = [
    "imeta",
    "url https://relay/media/a.png",
    "m image/png",
    "x abc",
    "size 12",
    "filename a.png",
  ];
  assert.deepEqual(
    attachmentDragContentAndTags({
      items: [
        {
          tag,
          url: "https://relay/media/a.png",
          type: "image/png",
          sha256: "abc",
          size: 12,
          uploaded: 0,
          filename: "a.png",
        },
      ],
    }),
    {
      content: "\n![image](https://relay/media/a.png)",
      mediaTags: [tag],
    },
  );
});
