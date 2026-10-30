import assert from "node:assert/strict";
import test from "node:test";

import { getContextMessageDepth } from "../../home/lib/inboxViewHelpers.ts";
import { formatTimelineMessages } from "./formatTimelineMessages.ts";
import {
  buildMainTimelineEntries,
  buildThreadPanelData,
} from "./threadPanel.ts";

const CHANNEL_ID = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const ROOT = "a".repeat(64);
const CHILD = "b".repeat(64);
const GRANDCHILD = "c".repeat(64);

function event(id, createdAt, threadTags = []) {
  return {
    id,
    pubkey: "1".repeat(64),
    kind: 9,
    created_at: createdAt,
    content: id.slice(0, 1),
    tags: [["h", CHANNEL_ID], ...threadTags],
    sig: "sig",
  };
}

// root <- child <- grandchild, as signed by a nested-era client (NIP-10).
const EVENTS = [
  event(ROOT, 1),
  event(CHILD, 2, [["e", ROOT, "", "reply"]]),
  event(GRANDCHILD, 3, [
    ["e", ROOT, "", "root"],
    ["e", CHILD, "", "reply"],
  ]),
];

function presentThread(threadPolicy) {
  const messages = formatTimelineMessages(
    EVENTS,
    null,
    undefined,
    null,
    undefined,
    undefined,
    undefined,
    undefined,
    null,
    undefined,
    threadPolicy,
  );
  const byId = new Map(messages.map((message) => [message.id, message]));
  const [rootEntry] = buildMainTimelineEntries(messages);
  const panel = buildThreadPanelData(messages, ROOT, ROOT, new Set());
  return {
    grandchild: byId.get(GRANDCHILD),
    mainTimelineIds: buildMainTimelineEntries(messages).map(
      (entry) => entry.message.id,
    ),
    rootSummaryCount: rootEntry.summary?.replyCount,
    totalReplyCount: panel.totalReplyCount,
    visibleReplyIds: panel.visibleReplies.map((entry) => entry.message.id),
  };
}

for (const { policy, expected } of [
  {
    policy: "nested",
    expected: {
      grandchildParentId: CHILD,
      grandchildDepth: 2,
      // The grandchild stays collapsed under its unexpanded parent.
      visibleReplyIds: [CHILD],
    },
  },
  {
    policy: "flat",
    expected: {
      grandchildParentId: ROOT,
      grandchildDepth: 1,
      visibleReplyIds: [CHILD, GRANDCHILD],
    },
  },
]) {
  test(`${policy} thread policy presents a nested-era thread`, () => {
    const thread = presentThread(policy);

    assert.equal(thread.grandchild.parentId, expected.grandchildParentId);
    assert.equal(thread.grandchild.rootId, ROOT);
    assert.equal(thread.grandchild.depth, expected.grandchildDepth);
    assert.deepEqual(thread.visibleReplyIds, expected.visibleReplyIds);
    // The channel row and thread header count every reply under either policy.
    assert.deepEqual(thread.mainTimelineIds, [ROOT]);
    assert.equal(thread.rootSummaryCount, 2);
    assert.equal(thread.totalReplyCount, 2);
  });
}

test("inbox context depth follows the thread policy", () => {
  const eventById = new Map(EVENTS.map((entry) => [entry.id, entry]));
  const grandchild = eventById.get(GRANDCHILD);

  assert.equal(getContextMessageDepth(grandchild, eventById), 2);
  assert.equal(getContextMessageDepth(grandchild, eventById, "nested"), 2);
  assert.equal(getContextMessageDepth(grandchild, eventById, "flat"), 1);
});
