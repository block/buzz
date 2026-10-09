import assert from "node:assert/strict";
import test from "node:test";

import { markVisibleThreadRepliesRead } from "./useChannelUnreadState.ts";

const threadMessages = [{ message: { id: "reply", createdAt: 42 } }];

test("a background open thread does not mark its visible replies read", () => {
  const calls = [];
  markVisibleThreadRepliesRead({
    appFocused: false,
    isThreadMuted: () => false,
    markMessageRead: (...args) => calls.push(args),
    openThreadHeadId: "root",
    threadMessages,
  });

  assert.deepEqual(calls, []);
});

test("focusing an open thread marks its visible replies read", () => {
  const calls = [];
  markVisibleThreadRepliesRead({
    appFocused: true,
    isThreadMuted: () => false,
    markMessageRead: (...args) => calls.push(args),
    openThreadHeadId: "root",
    threadMessages,
  });

  assert.deepEqual(calls, [["reply", 42]]);
});
