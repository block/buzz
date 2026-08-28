import assert from "node:assert/strict";
import test from "node:test";

import {
  collectMarkAllUnreadChannelIds,
  readAtForPreservedUnreadMessage,
} from "./unreadThreadEventIds.ts";

test("mark-all includes authoritative replies hidden for the active channel", () => {
  const projection = { unreadThreadEventIds: ["reply"] };
  assert.deepEqual(
    [
      ...collectMarkAllUnreadChannelIds(
        new Set(["visible"]),
        new Map([["active", projection]]),
      ),
    ],
    ["visible", "active"],
  );
});

test("authoritative unread id survives a newer revisit frontier", () => {
  const messageId = "reply";
  assert.equal(
    readAtForPreservedUnreadMessage(messageId, new Set([messageId]), null, 500),
    null,
  );
});

test("non-preserved message still folds the channel frontier", () => {
  assert.equal(
    readAtForPreservedUnreadMessage("reply", new Set(), 100, 500),
    500,
  );
});

test("late authoritative recovery remains unread after the channel opens", () => {
  const messageId = "reply";
  assert.equal(
    readAtForPreservedUnreadMessage(messageId, new Set([messageId]), null, 500),
    null,
  );
});
