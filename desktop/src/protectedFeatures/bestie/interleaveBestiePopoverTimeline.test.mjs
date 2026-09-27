import assert from "node:assert/strict";
import test from "node:test";

import { buildBestiePopoverTimelineRuns } from "./interleaveBestiePopoverTimeline.ts";

function message(partial) {
  return {
    author: partial.author ?? "user",
    body: partial.body ?? "hi",
    createdAt: partial.createdAt,
    depth: 0,
    id: partial.id,
    parentId: null,
    rootId: null,
    time: "12:00",
  };
}

function thought(partial) {
  return {
    id: partial.id,
    type: "thought",
    renderClass: "thought",
    title: "Thinking",
    text: partial.text ?? "…",
    timestamp: partial.timestamp,
    channelId: "dm-1",
  };
}

function tool(partial) {
  return {
    id: partial.id,
    type: "tool",
    renderClass: "file-read",
    descriptor: { renderClass: "file-read", label: "Searched", preview: null },
    title: "Searched",
    toolName: "Searched",
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp: partial.timestamp,
    startedAt: partial.timestamp,
    completedAt: partial.timestamp,
    channelId: "dm-1",
  };
}

test("interleave places activity before a later message by timestamp", () => {
  const runs = buildBestiePopoverTimelineRuns(
    [
      message({ createdAt: 100, id: "user-1" }),
      message({ createdAt: 130, id: "agent-1" }),
    ],
    [
      thought({ id: "think-1", timestamp: "1970-01-01T00:01:50.000Z" }), // 110s
      tool({ id: "search-1", timestamp: "1970-01-01T00:02:00.000Z" }), // 120s
    ],
  );

  assert.deepEqual(
    runs.map((run) =>
      run.kind === "messages"
        ? { kind: "messages", ids: run.messages.map((m) => m.id) }
        : { kind: "activity", ids: run.items.map((i) => i.id) },
    ),
    [
      { kind: "messages", ids: ["user-1"] },
      { kind: "activity", ids: ["think-1", "search-1"] },
      { kind: "messages", ids: ["agent-1"] },
    ],
  );
});

test("interleave keeps activity ahead of a message on equal timestamps", () => {
  const runs = buildBestiePopoverTimelineRuns(
    [message({ createdAt: 200, id: "agent-reply" })],
    [thought({ id: "think-same", timestamp: "1970-01-01T00:03:20.000Z" })],
  );

  assert.deepEqual(
    runs.map((run) => run.kind),
    ["activity", "messages"],
  );
  assert.equal(runs[0].kind === "activity" && runs[0].items[0].id, "think-same");
  assert.equal(
    runs[1].kind === "messages" && runs[1].messages[0].id,
    "agent-reply",
  );
});

test("interleave with no activity is a single message run", () => {
  const runs = buildBestiePopoverTimelineRuns(
    [
      message({ createdAt: 1, id: "a" }),
      message({ createdAt: 2, id: "b" }),
    ],
    [],
  );
  assert.deepEqual(runs, [
    {
      kind: "messages",
      messages: [
        message({ createdAt: 1, id: "a" }),
        message({ createdAt: 2, id: "b" }),
      ],
    },
  ]);
});

test("interleave with only activity is a single activity run", () => {
  const runs = buildBestiePopoverTimelineRuns(
    [],
    [thought({ id: "t1", timestamp: "1970-01-01T00:00:01.000Z" })],
  );
  assert.equal(runs.length, 1);
  assert.equal(runs[0].kind, "activity");
  assert.equal(runs[0].kind === "activity" && runs[0].items[0].id, "t1");
});
