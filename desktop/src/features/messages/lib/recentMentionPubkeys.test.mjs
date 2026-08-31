import assert from "node:assert/strict";
import test from "node:test";

import { getRecentMentionPubkeys } from "./recentMentionPubkeys.ts";

const AUTHOR = "a".repeat(64);
const OLDER = "1".repeat(64);
const LATEST_FIRST = "2".repeat(64);
const LATEST_LAST = "3".repeat(64);
const REPLY_AUTHOR = "4".repeat(64);

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

test("filters desktop structural self-tags by identity", () => {
  const parent = message(1, []);
  assert.deepEqual(
    getRecentMentionPubkeys([
      parent,
      {
        ...message(2, [
          ["p", REPLY_AUTHOR],
          ["p", LATEST_FIRST],
        ]),
        id: "desktop-reply",
        parentId: parent.id,
        pubkey: REPLY_AUTHOR,
      },
    ]),
    [LATEST_FIRST],
  );
});

test("keeps an sdk-shaped reply mention that matches the parent author", () => {
  const parent = message(1, []);
  assert.deepEqual(
    getRecentMentionPubkeys([
      parent,
      {
        ...message(2, [["p", AUTHOR]]),
        id: "sdk-reply",
        parentId: parent.id,
        pubkey: REPLY_AUTHOR,
      },
    ]),
    [AUTHOR],
  );
});

test("ignores DM participant fan-out tags", () => {
  assert.deepEqual(
    getRecentMentionPubkeys(
      [
        message(1, [
          ["p", AUTHOR],
          ["p", LATEST_FIRST],
          ["p", LATEST_LAST],
        ]),
      ],
      "dm",
    ),
    [],
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
