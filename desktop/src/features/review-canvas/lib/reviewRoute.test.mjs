import assert from "node:assert/strict";
import { test } from "node:test";

import {
  parseReviewNotification,
  parseReviewRevision,
} from "./reviewContract.ts";
import {
  reviewSearchFromNotification,
  validateReviewSearch,
  verifyAnnouncement,
} from "./reviewRoute.ts";
import {
  ADAPTER,
  AGENT,
  NOTIFICATION_ID,
  ORIGIN_EVENT,
  REVISION_EVENT,
  THREE_BLOCK_HTML,
  notificationMessage,
  reviewEvent,
  sha256,
} from "./reviewFixtures.mjs";

test("a notification opens its review carrying revision, digest, author, and thread", () => {
  const notification = parseReviewNotification(notificationMessage());
  assert.deepEqual(reviewSearchFromNotification(notification), {
    revision: REVISION_EVENT,
    agent: AGENT,
    digest: sha256(THREE_BLOCK_HTML),
    author: ADAPTER,
    notification: NOTIFICATION_ID,
    thread: ORIGIN_EVENT,
  });
});

test("route search keeps only well-formed 64-hex references", () => {
  assert.deepEqual(
    validateReviewSearch({
      revision: REVISION_EVENT,
      agent: "not-hex",
      digest: "A".repeat(64),
      author: 42,
      notification: NOTIFICATION_ID,
      thread: ["x"],
      extra: REVISION_EVENT,
    }),
    { revision: REVISION_EVENT, notification: NOTIFICATION_ID },
  );
  assert.deepEqual(validateReviewSearch({}), {});
});

test("a revision matching its notification verifies; any drift is refused", () => {
  const revision = parseReviewRevision(reviewEvent());
  assert.doesNotThrow(() =>
    verifyAnnouncement(revision, {
      author: ADAPTER,
      digest: sha256(THREE_BLOCK_HTML),
    }),
  );
  assert.doesNotThrow(() => verifyAnnouncement(revision, {}));

  assert.throws(
    () => verifyAnnouncement(revision, { digest: "9".repeat(64) }),
    /payload digest changed/,
  );
  assert.throws(
    () => verifyAnnouncement(revision, { author: "9".repeat(64) }),
    /not published by the account/,
  );
});
