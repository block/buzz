import assert from "node:assert/strict";
import { test } from "node:test";

import {
  EMPTY_PROJECT_SELECTION,
  mergeSelectionDiscussDraft,
  nextProjectGroupSelection,
  nextProjectSelection,
  projectSelectionChannelCandidates,
  projectSelectionChannelId,
  projectSelectionDiscussContent,
  projectSelectionPresentation,
  projectSelectionTitle,
  selectionItemFromCommit,
  selectionItemFromTask,
} from "./projectSelection.ts";

const taskA = selectionItemFromTask({
  author: "aa",
  channelId: "channel-1",
  id: "issue-a",
  shareLink: "buzz://issue?id=a",
  title: "Fix login",
});
const taskB = selectionItemFromTask({
  author: "bb",
  channelId: "channel-1",
  id: "issue-b",
  shareLink: "buzz://issue?id=b",
  title: "Fix logout",
});
const taskC = selectionItemFromTask({
  author: "cc",
  channelId: "channel-2",
  id: "issue-c",
  shareLink: "buzz://issue?id=c",
  title: "Fix signup",
});
const commit = selectionItemFromCommit({
  channelId: "channel-1",
  commitHash: "abc",
  projectId: "buzz",
  shareLink: "buzz://commit?h=abc",
  title: "Ship the pod",
});

test("selecting an item starts a cluster", () => {
  const next = nextProjectSelection(EMPTY_PROJECT_SELECTION, taskA);
  assert.deepEqual(
    next.items.map((item) => item.id),
    [taskA.id],
  );
  assert.equal(next.anchorId, taskA.id);
});

test("selecting the same item again removes it", () => {
  const selected = nextProjectSelection(EMPTY_PROJECT_SELECTION, taskA);
  const next = nextProjectSelection(selected, taskA);
  assert.deepEqual(next.items, []);
  assert.equal(next.anchorId, null);
});

test("selecting a different kind replaces the cluster", () => {
  const selected = nextProjectSelection(EMPTY_PROJECT_SELECTION, taskA);
  const next = nextProjectSelection(selected, commit);
  assert.deepEqual(
    next.items.map((item) => item.id),
    [commit.id],
  );
});

test("shift-click selects a same-kind range", () => {
  const selected = nextProjectSelection(EMPTY_PROJECT_SELECTION, taskA);
  const next = nextProjectSelection(selected, taskC, {
    rangeItems: [taskA, taskB, taskC],
    shiftKey: true,
  });
  assert.deepEqual(
    next.items.map((item) => item.id),
    [taskA.id, taskB.id, taskC.id],
  );
  assert.equal(next.anchorId, taskA.id);
});

test("group selection adds every item and preserves selections outside the group", () => {
  const selected = nextProjectSelection(EMPTY_PROJECT_SELECTION, taskC);
  const next = nextProjectGroupSelection(selected, [taskA, taskB]);
  assert.deepEqual(
    next.items.map((item) => item.id),
    [taskC.id, taskA.id, taskB.id],
  );
});

test("group selection clears the group when every item is selected", () => {
  const selected = nextProjectGroupSelection(EMPTY_PROJECT_SELECTION, [
    taskA,
    taskB,
  ]);
  const next = nextProjectGroupSelection(selected, [taskA, taskB]);
  assert.deepEqual(next, EMPTY_PROJECT_SELECTION);
});

test("selection presentation names the cluster and its actions", () => {
  const presentation = projectSelectionPresentation([taskA, taskB]);
  assert.equal(presentation?.title, "2 tasks");
  assert.deepEqual(
    presentation?.actions.map((action) => [action.id, action.label]),
    [
      ["chat-agent", "Chat with an agent"],
      ["discuss", "Discuss in a channel"],
      ["copy", "Copy links"],
    ],
  );
  assert.deepEqual(presentation?.people, ["aa", "bb"]);
});

test("commit clusters offer create-review instead of a generic copy-only set", () => {
  const presentation = projectSelectionPresentation([commit]);
  assert.equal(presentation?.title, "1 commit");
  assert.deepEqual(
    presentation?.actions.map((action) => action.id),
    ["chat-agent", "discuss", "create-review", "copy"],
  );
});

test("discuss content prefers share links and the majority channel", () => {
  assert.equal(projectSelectionChannelId([taskA, taskB, taskC]), "channel-1");
  assert.deepEqual(projectSelectionChannelCandidates([taskA, taskB, taskC]), [
    { channelId: "channel-1", count: 2 },
    { channelId: "channel-2", count: 1 },
  ]);
  assert.equal(
    projectSelectionDiscussContent([taskA, taskB]),
    "Let's talk about these tasks:\n\nbuzz://issue?id=a\nbuzz://issue?id=b",
  );
  assert.equal(projectSelectionTitle([commit]), "1 commit");
});

test("discussion remains available when channel search is the only choice", () => {
  const item = selectionItemFromTask({ id: "unbound", title: "Unbound task" });
  assert.equal(
    projectSelectionPresentation([item])?.actions.some(
      (action) => action.id === "discuss",
    ),
    true,
  );
  assert.deepEqual(projectSelectionChannelCandidates([item]), []);
});

test("discuss drafts append instead of replacing an existing composer draft", () => {
  assert.equal(
    mergeSelectionDiscussDraft("hello", "Let's talk about this task:\n\nlink"),
    "hello\n\nLet's talk about this task:\n\nlink",
  );
  assert.equal(
    mergeSelectionDiscussDraft(
      "Let's talk about this task:\n\nlink",
      "Let's talk about this task:\n\nlink",
    ),
    "Let's talk about this task:\n\nlink",
  );
});
