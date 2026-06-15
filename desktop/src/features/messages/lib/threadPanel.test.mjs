import assert from "node:assert/strict";
import test from "node:test";

import {
  buildMainTimelineEntries,
  buildThreadPanelData,
  buildThreadPanelDataFromIndex,
  buildThreadPanelIndex,
} from "./threadPanel.ts";

function message(overrides) {
  return {
    id: "message",
    createdAt: 1,
    pubkey: "author",
    author: "Author",
    avatarUrl: null,
    role: undefined,
    personaDisplayName: undefined,
    time: "12:00 PM",
    body: "body",
    parentId: null,
    rootId: null,
    depth: 0,
    accent: false,
    pending: undefined,
    edited: false,
    kind: 9,
    tags: [],
    reactions: undefined,
    ...overrides,
  };
}

test("buildMainTimelineEntries includes broadcast replies", () => {
  const root = message({ id: "root", createdAt: 1 });
  const hiddenReply = message({
    id: "hidden-reply",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
    tags: [["e", "root", "", "reply"]],
  });
  const broadcastReply = message({
    id: "broadcast-reply",
    createdAt: 3,
    parentId: "root",
    rootId: "root",
    depth: 1,
    tags: [
      ["e", "root", "", "reply"],
      ["broadcast", "1"],
    ],
  });

  assert.deepEqual(
    buildMainTimelineEntries([root, hiddenReply, broadcastReply]).map(
      (entry) => entry.message.id,
    ),
    ["root", "broadcast-reply"],
  );
});

test("buildThreadPanelData keeps direct comments unindented", () => {
  const root = message({ id: "root", createdAt: 1 });
  const directComment = message({
    id: "direct-comment",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
    tags: [["e", "root", "", "reply"]],
  });
  const nestedReply = message({
    id: "nested-reply",
    createdAt: 3,
    parentId: "direct-comment",
    rootId: "root",
    depth: 2,
    tags: [
      ["e", "root", "", "root"],
      ["e", "direct-comment", "", "reply"],
    ],
  });

  const panelData = buildThreadPanelData(
    [root, directComment, nestedReply],
    "root",
    "root",
    new Set(["direct-comment"]),
  );

  assert.deepEqual(
    panelData.visibleReplies.map((entry) => ({
      id: entry.message.id,
      depth: entry.message.depth,
    })),
    [
      { id: "direct-comment", depth: 0 },
      { id: "nested-reply", depth: 1 },
    ],
  );
});

// Per-id stabilization: thread rows feed `MessageRow` a depth-normalized copy
// of each reply. When `timelineMessages` churns (typing/presence) but the
// reply objects survive by reference, rebuilding the thread panel must hand
// `MessageRow` the SAME normalized object reference so the row/markdown memo
// hits — instead of a fresh `{ ...reply, depth }` spread every render.
test("thread reply objects keep identity across unrelated timelineMessages churn", () => {
  const root = message({ id: "root", createdAt: 1 });
  const replyA = message({
    id: "a",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
    tags: [["e", "root", "", "reply"]],
  });
  const replyB = message({
    id: "b",
    createdAt: 3,
    parentId: "a",
    rootId: "root",
    depth: 2,
    tags: [["e", "a", "", "reply"]],
  });

  // First render of the thread.
  const first = buildThreadPanelData(
    [root, replyA, replyB],
    "root",
    "root",
    new Set(["a"]),
  );

  // An unrelated channel churn produces a NEW `timelineMessages` array, but the
  // reply objects themselves are reused by reference (only their position in
  // the surrounding array changed — e.g. a presence ping or typing indicator
  // that the snapshot layer leaves the reply identities intact for).
  const churned = [
    message({ id: "noise", createdAt: 99 }),
    root,
    replyA,
    replyB,
  ];
  const second = buildThreadPanelData(churned, "root", "root", new Set(["a"]));

  const firstById = new Map(
    first.visibleReplies.map((entry) => [entry.message.id, entry.message]),
  );
  const secondById = new Map(
    second.visibleReplies.map((entry) => [entry.message.id, entry.message]),
  );

  assert.ok(firstById.size > 0, "expected at least one visible reply");
  for (const [id, normalized] of firstById) {
    assert.strictEqual(
      secondById.get(id),
      normalized,
      `normalized reply ${id} must be the SAME object reference across an unrelated churn (memo hit)`,
    );
    // Depth must still reach the row correctly via the cached object.
    assert.equal(
      typeof normalized.depth,
      "number",
      `normalized reply ${id} must carry a numeric depth`,
    );
  }
});

test("thread reply objects recompute when the source reply object is replaced", () => {
  const root = message({ id: "root", createdAt: 1 });
  const reply = message({
    id: "a",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
    tags: [["e", "root", "", "reply"]],
  });

  const first = buildThreadPanelData([root, reply], "root", "root", new Set());

  // A genuine edit/refresh: the reply is a brand-new object (new identity).
  const editedReply = message({
    id: "a",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
    body: "edited body",
    tags: [["e", "root", "", "reply"]],
  });
  const second = buildThreadPanelData(
    [root, editedReply],
    "root",
    "root",
    new Set(),
  );

  const firstA = first.visibleReplies.find((e) => e.message.id === "a");
  const secondA = second.visibleReplies.find((e) => e.message.id === "a");
  assert.ok(firstA && secondA, "expected reply 'a' in both renders");
  assert.notStrictEqual(
    secondA.message,
    firstA.message,
    "a replaced source reply must produce a fresh normalized object",
  );
  assert.equal(secondA.message.body, "edited body");
});

test("buildThreadPanelDataFromIndex matches direct panel data", () => {
  const root = message({ id: "root", createdAt: 1 });
  const directComment = message({
    id: "direct-comment",
    createdAt: 2,
    parentId: "root",
    rootId: "root",
    depth: 1,
    tags: [["e", "root", "", "reply"]],
  });
  const nestedReply = message({
    id: "nested-reply",
    createdAt: 3,
    parentId: "direct-comment",
    rootId: "root",
    depth: 2,
    tags: [
      ["e", "root", "", "root"],
      ["e", "direct-comment", "", "reply"],
    ],
  });
  const messages = [root, directComment, nestedReply];

  const direct = buildThreadPanelData(
    messages,
    "root",
    "direct-comment",
    new Set(["direct-comment"]),
  );
  const indexed = buildThreadPanelDataFromIndex(
    buildThreadPanelIndex(messages),
    "root",
    "direct-comment",
    new Set(["direct-comment"]),
  );

  assert.deepEqual(indexed, direct);
});
