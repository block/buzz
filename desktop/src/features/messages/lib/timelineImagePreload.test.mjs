import assert from "node:assert/strict";
import { test } from "node:test";

import { timelineImageUrls } from "./timelineImagePreload.ts";

function message(over = {}) {
  return {
    id: "m1",
    createdAt: 0,
    author: "a",
    time: "now",
    body: "",
    depth: 0,
    ...over,
  };
}

test("timelineImageUrls warms chrome images but leaves attachments lazy", () => {
  const urls = timelineImageUrls(
    message({
      avatarUrl: "https://example.com/avatar.jpg",
      body: [
        "![first](https://example.com/one.png)",
        '![second](https://example.com/two.webp "caption")',
      ].join("\n"),
      tags: [
        ["imeta", "url https://example.com/three.jpg", "m image/jpeg"],
        [
          "imeta",
          "url https://example.com/movie.mp4",
          "m video/mp4",
          "image https://example.com/poster.jpg",
          "thumb https://example.com/thumb.jpg",
        ],
        ["emoji", "party", "https://example.com/party.png"],
      ],
      reactions: [
        {
          emoji: ":party:",
          emojiUrl: "https://example.com/reaction.png",
          count: 1,
          users: [],
        },
      ],
    }),
  );

  assert.deepEqual(
    new Set(urls),
    new Set([
      "https://example.com/avatar.jpg",
      "https://example.com/poster.jpg",
      "https://example.com/thumb.jpg",
      "https://example.com/party.png",
      "https://example.com/reaction.png",
    ]),
  );
  assert.ok(!urls.includes("https://example.com/one.png"));
  assert.ok(!urls.includes("https://example.com/three.jpg"));
});

test("timelineImageUrls deduplicates chrome image URLs", () => {
  const url = "https://example.com/same.png";
  assert.deepEqual(
    timelineImageUrls(
      message({
        avatarUrl: url,
        tags: [["emoji", "same", url]],
      }),
    ),
    [url],
  );
});

test("preloadTimelineImages requests URLs once and keeps requests alive", async () => {
  const { preloadTimelineImages } = await import("./timelineImagePreload.ts");
  const previousImage = globalThis.Image;
  const requested = [];
  class FakeImage extends EventTarget {
    set src(url) {
      requested.push(url);
    }
  }
  globalThis.Image = FakeImage;
  try {
    const state = { activeImages: new Set(), requestedUrls: new Set() };
    const messages = [message({ avatarUrl: "https://example.com/one.png" })];
    preloadTimelineImages(messages, state);
    preloadTimelineImages(messages, state);

    assert.deepEqual(requested, ["https://example.com/one.png"]);
    assert.equal(state.activeImages.size, 1);
    state.activeImages.values().next().value.dispatchEvent(new Event("load"));
    assert.equal(state.activeImages.size, 0);
  } finally {
    globalThis.Image = previousImage;
  }
});
