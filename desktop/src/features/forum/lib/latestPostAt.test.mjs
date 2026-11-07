import assert from "node:assert/strict";
import test from "node:test";

import { latestForumPostAt } from "./latestPostAt.ts";

const page = (...createdAts) => ({
  posts: createdAts.map((createdAt, index) => ({
    eventId: `e${index}`,
    pubkey: "a",
    content: "c",
    kind: 45001,
    createdAt,
    channelId: "ch",
    tags: [],
    threadSummary: null,
  })),
  nextCursor: null,
});

test("returns the newest post across every loaded page", () => {
  assert.equal(latestForumPostAt([page(30, 20), page(40, 10)]), 40);
});

test("no pages and empty pages yield null", () => {
  assert.equal(latestForumPostAt(undefined), null);
  assert.equal(latestForumPostAt([]), null);
  assert.equal(latestForumPostAt([page()]), null);
});

test("a single post is its own latest", () => {
  assert.equal(latestForumPostAt([page(7)]), 7);
});
