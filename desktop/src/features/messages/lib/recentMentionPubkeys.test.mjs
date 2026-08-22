import assert from "node:assert/strict";
import test from "node:test";

import { getRecentMentionPubkeys } from "./recentMentionPubkeys.ts";

const AUTHOR = "a".repeat(64);
const OLDER = "1".repeat(64);
const LATEST_FIRST = "2".repeat(64);
const LATEST_LAST = "3".repeat(64);

function message(createdAt, tags) {
  return {
    id: String(createdAt),
    createdAt,
    pubkey: AUTHOR,
    author: "Author",
    time: "",
    body: "",
    depth: 0,
    tags,
  };
}

test("returns loaded channel mentions newest-first and excludes structural author tags", () => {
  assert.deepEqual(
    getRecentMentionPubkeys([
      message(1, [
        ["p", AUTHOR],
        ["p", OLDER],
      ]),
      message(2, [
        ["p", AUTHOR],
        ["p", LATEST_FIRST],
        ["p", LATEST_LAST],
      ]),
    ]),
    [LATEST_LAST, LATEST_FIRST, OLDER],
  );
});

test("keeps a top-level mention when the event omits its structural author tag", () => {
  assert.deepEqual(
    getRecentMentionPubkeys([message(1, [["p", LATEST_FIRST]])]),
    [LATEST_FIRST],
  );
});

test("excludes the first p tag on replies", () => {
  assert.deepEqual(
    getRecentMentionPubkeys([
      {
        ...message(1, [
          ["p", OLDER],
          ["p", LATEST_FIRST],
        ]),
        parentId: "root",
      },
    ]),
    [LATEST_FIRST],
  );
});

test("deduplicates repeated mentions at their newest position", () => {
  assert.deepEqual(
    getRecentMentionPubkeys([
      message(1, [
        ["p", AUTHOR],
        ["p", LATEST_FIRST],
      ]),
      message(2, [
        ["p", AUTHOR],
        ["p", LATEST_FIRST],
      ]),
    ]),
    [LATEST_FIRST],
  );
});
