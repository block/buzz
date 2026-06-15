import assert from "node:assert/strict";
import test from "node:test";

import {
  buildDescendantStatsByMessageId,
  buildMainTimelineEntries,
  buildThreadPanelData,
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

// A.3.1 regression microbench: the channel-wide descendant walk must run ONCE
// per `timelineMessages` change, not once per thread-open. The ChannelScreen
// memo computes `buildDescendantStatsByMessageId(timelineMessages)` keyed on the
// message set alone, then shares the result with every `buildThreadPanelData`
// call. This test mirrors that contract and counts the heavy walks.
function buildChannel(messageCount, branchingDepth) {
  const messages = [message({ id: "root", createdAt: 1 })];
  let parentId = "root";
  for (let index = 1; index < messageCount; index += 1) {
    const id = `m${index}`;
    messages.push(
      message({
        id,
        createdAt: index + 1,
        parentId,
        rootId: "root",
        depth: 1,
        tags: [["e", parentId, "", "reply"]],
      }),
    );
    // Re-anchor to root every `branchingDepth` to vary the tree shape.
    parentId = index % branchingDepth === 0 ? "root" : id;
  }
  return messages;
}

test("A.3.1: channel-wide walk runs once per timelineMessages change, not per thread-open", () => {
  const messages = buildChannel(200, 5);

  // Count how many times the expensive whole-channel walk actually fires.
  let walkCount = 0;
  const countingBuildStats = (msgs) => {
    walkCount += 1;
    return buildDescendantStatsByMessageId(msgs);
  };

  // The ChannelScreen seam: compute the channel-wide stats ONCE for this
  // `timelineMessages` identity...
  const sharedStats = countingBuildStats(messages);

  // ...then drive many thread-opens / expands reusing the shared map. None of
  // these should re-walk the whole channel.
  const threadOpenIds = ["root", "m5", "m10", "m25", "m50", "m100"];
  const results = threadOpenIds.map((openThreadHeadId) =>
    buildThreadPanelData(
      messages,
      openThreadHeadId,
      openThreadHeadId,
      new Set(),
      sharedStats,
    ),
  );

  // Exactly one whole-channel walk despite 6 thread-opens.
  assert.equal(
    walkCount,
    1,
    `expected 1 channel-wide walk for ${threadOpenIds.length} thread-opens, got ${walkCount}`,
  );

  // The shared-stats path must produce identical output to the
  // build-it-internally path (back-compat: omitting the arg recomputes).
  for (let index = 0; index < threadOpenIds.length; index += 1) {
    const openThreadHeadId = threadOpenIds[index];
    const recomputed = buildThreadPanelData(
      messages,
      openThreadHeadId,
      openThreadHeadId,
      new Set(),
    );
    assert.equal(
      results[index].totalReplyCount,
      recomputed.totalReplyCount,
      `totalReplyCount mismatch for thread ${openThreadHeadId}`,
    );
    assert.deepEqual(
      results[index].visibleReplies.map((entry) => entry.message.id),
      recomputed.visibleReplies.map((entry) => entry.message.id),
      `visibleReplies mismatch for thread ${openThreadHeadId}`,
    );
  }

  // The main-timeline path shares the same map too — still one walk total.
  const mainEntries = buildMainTimelineEntries(messages, sharedStats);
  assert.equal(
    walkCount,
    1,
    "buildMainTimelineEntries must reuse the shared stats, not re-walk",
  );
  assert.ok(mainEntries.length > 0);
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
