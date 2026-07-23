import assert from "node:assert/strict";
import test from "node:test";

import { resolveInboxOpenScrollTarget } from "./inboxOpenScroll.ts";

const messages = [
  { createdAt: 10, id: "root" },
  { createdAt: 20, id: "read-reply" },
  { createdAt: 30, id: "first-unread" },
  { createdAt: 40, id: "latest" },
];

test("unread thread opens at the first reply beyond its captured frontier", () => {
  assert.deepEqual(
    resolveInboxOpenScrollTarget(
      {
        anchorEventId: "latest",
        conversationId: "root",
        excludedRootId: "root",
        forcedUnreadMessageId: null,
        openReadAt: 20,
        requestId: 1,
        wasUnread: true,
      },
      messages,
    ),
    { alignment: "top-with-divider", id: "first-unread" },
  );
});

test("already-read thread opens at its physical bottom", () => {
  assert.deepEqual(
    resolveInboxOpenScrollTarget(
      {
        anchorEventId: "latest",
        conversationId: "root",
        excludedRootId: "root",
        forcedUnreadMessageId: null,
        openReadAt: 40,
        requestId: 2,
        wasUnread: false,
      },
      messages,
    ),
    { alignment: "bottom", id: "latest" },
  );
});

test("local mark-unread targets the overridden representative event", () => {
  assert.deepEqual(
    resolveInboxOpenScrollTarget(
      {
        anchorEventId: "latest",
        conversationId: "root",
        excludedRootId: "root",
        forcedUnreadMessageId: "latest",
        openReadAt: 40,
        requestId: 3,
        wasUnread: true,
      },
      messages,
    ),
    { alignment: "top-with-divider", id: "latest" },
  );
});
