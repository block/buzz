import assert from "node:assert/strict";
import test from "node:test";

import {
  isBestiePopoverSystemNoise,
  previewBestiePopoverMessageBody,
  resolveBestiePopoverNewMessageQueue,
  resolveBestiePopoverNewMessageTarget,
} from "./bestiePopoverNewMessage.ts";

test("outside-session message wins even when near bottom", () => {
  const target = resolveBestiePopoverNewMessageTarget({
    allMessages: [
      { id: "in", createdAt: 10, body: "session" },
      { id: "out", createdAt: 20, body: "elsewhere" },
    ],
    sessionMessageIds: new Set(["in"]),
    sessionRootId: "in",
    nearBottom: true,
  });
  assert.deepEqual(target, {
    id: "out",
    outsideSession: true,
    preview: "elsewhere",
    threadRootId: "out",
  });
});

test("in-session below fold offers jump when not near bottom", () => {
  const target = resolveBestiePopoverNewMessageTarget({
    allMessages: [
      { id: "a", createdAt: 10, body: "old" },
      { id: "b", createdAt: 20, body: "new below" },
    ],
    sessionMessageIds: new Set(["a", "b"]),
    sessionRootId: "a",
    nearBottom: false,
  });
  assert.deepEqual(target, {
    id: "b",
    outsideSession: false,
    preview: "new below",
    threadRootId: "b",
  });
});

test("near bottom with only in-session messages hides banner", () => {
  const target = resolveBestiePopoverNewMessageTarget({
    allMessages: [{ id: "a", createdAt: 10, body: "hi" }],
    sessionMessageIds: new Set(["a"]),
    sessionRootId: "a",
    nearBottom: true,
  });
  assert.equal(target, null);
});

test("preview collapses whitespace and truncates", () => {
  assert.equal(previewBestiePopoverMessageBody("  hello   world  "), "hello world");
  assert.equal(previewBestiePopoverMessageBody("abcdefghij", 6), "abcde…");
  assert.equal(previewBestiePopoverMessageBody(null), "");
});

test("queue one entry per outside-session thread, newest first", () => {
  const queue = resolveBestiePopoverNewMessageQueue({
    allMessages: [
      { id: "t1-old", createdAt: 10, body: "t1 old", rootId: "t1" },
      { id: "t1-new", createdAt: 30, body: "t1 new", rootId: "t1" },
      { id: "t2", createdAt: 20, body: "t2 only", rootId: "t2" },
      { id: "in", createdAt: 40, body: "in session" },
    ],
    sessionMessageIds: new Set(["in"]),
    sessionRootId: "in",
    nearBottom: true,
  });
  assert.deepEqual(
    queue.map((item) => ({ id: item.id, threadRootId: item.threadRootId, preview: item.preview })),
    [
      { id: "t1-new", threadRootId: "t1", preview: "t1 new" },
      { id: "t2", threadRootId: "t2", preview: "t2 only" },
    ],
  );
});

test("dismissed message ids skip that entry until a newer one arrives", () => {
  const queue = resolveBestiePopoverNewMessageQueue({
    allMessages: [
      { id: "t1-old", createdAt: 10, body: "old", rootId: "t1" },
      { id: "t1-new", createdAt: 30, body: "new", rootId: "t1" },
      { id: "t2", createdAt: 20, body: "other", rootId: "t2" },
    ],
    sessionMessageIds: new Set(["session"]),
    sessionRootId: "session",
    nearBottom: true,
    dismissedMessageIds: new Set(["t1-new"]),
  });
  // t1-new dismissed → t1-old is older in same thread and skipped by thread
  // grouping (newest-per-thread was t1-new). So only t2 remains.
  assert.deepEqual(
    queue.map((item) => item.id),
    ["t2"],
  );

  const withOlderDismissed = resolveBestiePopoverNewMessageQueue({
    allMessages: [
      { id: "t1-old", createdAt: 10, body: "old", rootId: "t1" },
      { id: "t1-new", createdAt: 30, body: "new", rootId: "t1" },
      { id: "session", createdAt: 5, body: "in" },
    ],
    sessionMessageIds: new Set(["session"]),
    sessionRootId: "session",
    nearBottom: true,
    dismissedMessageIds: new Set(["t1-old"]),
  });
  assert.deepEqual(
    withOlderDismissed.map((item) => item.id),
    ["t1-new"],
  );
});

test("dismissed thread root stays clear", () => {
  const queue = resolveBestiePopoverNewMessageQueue({
    allMessages: [
      { id: "t1-new", createdAt: 30, body: "new", rootId: "t1" },
      { id: "session", createdAt: 5, body: "in" },
    ],
    sessionMessageIds: new Set(["session"]),
    sessionRootId: "session",
    nearBottom: true,
    dismissedThreadRootIds: new Set(["t1"]),
  });
  assert.deepEqual(queue.map((item) => item.id), []);
});

test("baseline and pre-session messages are not new", () => {
  const queue = resolveBestiePopoverNewMessageQueue({
    allMessages: [
      { id: "old", createdAt: 5, body: "history" },
      { id: "base", createdAt: 8, body: "baseline" },
      { id: "fresh", createdAt: 20, body: "job reply" },
      { id: "session", createdAt: 10, body: "hi" },
    ],
    sessionMessageIds: new Set(["session"]),
    sessionRootId: "session",
    nearBottom: true,
    baselineMessageIds: new Set(["base", "old"]),
    minCreatedAt: 10,
  });
  assert.deepEqual(
    queue.map((item) => item.id),
    ["fresh"],
  );
});

test("no active session does not flood with history", () => {
  const queue = resolveBestiePopoverNewMessageQueue({
    allMessages: [
      { id: "a", createdAt: 1, body: "old1" },
      { id: "b", createdAt: 2, body: "old2" },
    ],
    sessionMessageIds: new Set(),
    nearBottom: true,
  });
  assert.deepEqual(queue, []);
});

test("Bestie system noise is skipped", () => {
  assert.equal(isBestiePopoverSystemNoise("[Bestie coffee]\n\n/hula-coffee"), true);
  assert.equal(isBestiePopoverSystemNoise("[Bestie job: x]\n\nprompt"), true);
  assert.equal(isBestiePopoverSystemNoise("[Bestie reminder]\n\ndue"), true);
  assert.equal(isBestiePopoverSystemNoise("normal agent reply"), false);

  const queue = resolveBestiePopoverNewMessageQueue({
    allMessages: [
      { id: "coffee", createdAt: 30, body: "[Bestie coffee]\n\n/hula-coffee" },
      { id: "real", createdAt: 20, body: "hey look at this" },
      { id: "session", createdAt: 5, body: "in" },
    ],
    sessionMessageIds: new Set(["session"]),
    sessionRootId: "session",
    nearBottom: true,
  });
  assert.deepEqual(
    queue.map((item) => item.id),
    ["real"],
  );
});

test("parentId falls back as thread root", () => {
  const target = resolveBestiePopoverNewMessageTarget({
    allMessages: [
      { id: "reply", createdAt: 5, body: "reply body", parentId: "root-1" },
      { id: "session", createdAt: 1, body: "in" },
    ],
    sessionMessageIds: new Set(["session"]),
    sessionRootId: "session",
    nearBottom: true,
  });
  assert.equal(target?.threadRootId, "root-1");
  assert.equal(target?.preview, "reply body");
});
