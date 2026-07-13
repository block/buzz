import assert from "node:assert/strict";
import test from "node:test";

import {
  buildTranscriptDisplayBlocks,
  flattenDisplayBlocks,
  formatTurnSetupLabel,
} from "./agentSessionTranscriptGrouping.ts";
import { isObserverEventAfter } from "../observerRelayStore.ts";

const baseTimestamp = "2026-06-14T22:20:23.000Z";

function lifecycle(id, title, acpSource, turnId, text = "") {
  return {
    id,
    type: "lifecycle",
    title,
    text,
    timestamp: baseTimestamp,
    acpSource,
    turnId,
    sessionId: "sess-1",
    channelId: "channel-1",
  };
}

function userPrompt(id, text, turnId) {
  return {
    id,
    type: "message",
    role: "user",
    title: "Buzz event",
    text,
    timestamp: baseTimestamp,
    acpSource: "session/prompt:user",
    turnId,
    sessionId: "sess-1",
    channelId: "channel-1",
  };
}

function promptContext(id, turnId) {
  return {
    id,
    type: "metadata",
    title: "Prompt context",
    sections: [{ title: "Channel", body: "general" }],
    timestamp: baseTimestamp,
    acpSource: "session/prompt:context",
    turnId,
    sessionId: "sess-1",
    channelId: "channel-1",
  };
}

function assistantMessage(id, text, turnId) {
  return {
    id,
    type: "message",
    role: "assistant",
    title: "Assistant",
    text,
    timestamp: "2026-06-14T22:20:47.000Z",
    acpSource: "agent_message_chunk",
    turnId,
    sessionId: "sess-1",
    channelId: "channel-1",
  };
}

function toolCall(id, turnId) {
  return {
    id,
    type: "tool",
    title: "Shell",
    toolName: "buzz-dev-mcp__shell",
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "ok",
    isError: false,
    timestamp: "2026-06-14T22:20:47.000Z",
    startedAt: "2026-06-14T22:20:47.000Z",
    completedAt: "2026-06-14T22:20:47.400Z",
    acpSource: "tool_call_update",
    turnId,
    sessionId: "sess-1",
    channelId: "channel-1",
  };
}

test("buildTranscriptDisplayBlocks bundles user prompt, setup, and context together", () => {
  const rawItems = [
    lifecycle(
      "turn",
      "Turn started",
      "turn_started",
      "turn-1",
      "Triggered by 1 event.",
    ),
    lifecycle("session", "Session ready", "session_resolved", "turn-1"),
    userPrompt("prompt", "@Ned deliberate, wider pass", "turn-1"),
    promptContext("context", "turn-1"),
    assistantMessage("assistant", "Thinking out loud.", "turn-1"),
    toolCall("tool", "turn-1"),
  ];

  const blocks = buildTranscriptDisplayBlocks(rawItems);
  const displayOrder = flattenDisplayBlocks(blocks).map((item) => item.id);

  assert.deepEqual(displayOrder, [
    "prompt",
    "turn",
    "session",
    "context",
    "assistant",
    "tool",
  ]);

  const turnBlock = blocks[0];
  assert.equal(turnBlock?.kind, "turn");
  assert.equal(turnBlock.segments[0]?.kind, "prompt");
  const promptSegment = turnBlock.segments[0];
  assert.equal(promptSegment.user.id, "prompt");
  assert.equal(promptSegment.context?.id, "context");
  assert.equal(promptSegment.setup.length, 2);
  assert.equal(turnBlock.segments[1]?.kind, "item");
  assert.equal(turnBlock.segments[2]?.kind, "item");
});

test("buildTranscriptDisplayBlocks collapses setup lifecycle inside prompt bundle", () => {
  const rawItems = [
    lifecycle("turn", "Turn started", "turn_started", "turn-1"),
    lifecycle("session", "Session ready", "session_resolved", "turn-1"),
    userPrompt("prompt", "hello", "turn-1"),
  ];

  const blocks = buildTranscriptDisplayBlocks(rawItems);
  assert.equal(blocks.length, 1);
  assert.equal(blocks[0]?.kind, "turn");

  const turnBlock = blocks[0];
  assert.equal(turnBlock.segments.length, 1);
  assert.equal(turnBlock.segments[0]?.kind, "prompt");
  assert.equal(
    formatTurnSetupLabel(turnBlock.segments[0].setup),
    "Turn started · Session ready",
  );
});

test("buildTranscriptDisplayBlocks hides setup and context when prompt is missing", () => {
  const rawItems = [
    lifecycle("turn", "Turn started", "turn_started", "turn-1"),
    lifecycle("session", "Session ready", "session_resolved", "turn-1"),
    promptContext("context", "turn-1"),
    toolCall("tool", "turn-1"),
  ];

  const blocks = buildTranscriptDisplayBlocks(rawItems);
  const displayOrder = flattenDisplayBlocks(blocks).map((item) => item.id);

  assert.deepEqual(displayOrder, ["tool"]);
});

test("buildTranscriptDisplayBlocks drops setup-and-context-only turns", () => {
  const rawItems = [
    lifecycle("turn", "Turn started", "turn_started", "turn-1"),
    lifecycle("session", "Session ready", "session_resolved", "turn-1"),
    promptContext("context", "turn-1"),
  ];

  const blocks = buildTranscriptDisplayBlocks(rawItems);

  assert.deepEqual(blocks, []);
});

test("buildTranscriptDisplayBlocks leaves error lifecycle prominent outside prompt bundle", () => {
  const rawItems = [
    lifecycle("turn", "Turn started", "turn_started", "turn-1"),
    userPrompt("prompt", "hello", "turn-1"),
    lifecycle(
      "error",
      "Turn error",
      "turn_error",
      "turn-1",
      "timeout: agent hung",
    ),
  ];

  const blocks = buildTranscriptDisplayBlocks(rawItems);
  const displayOrder = flattenDisplayBlocks(blocks).map((item) => item.id);

  assert.deepEqual(displayOrder, ["prompt", "turn", "error"]);
  assert.equal(blocks[0]?.segments[0]?.kind, "prompt");
  assert.equal(blocks[0]?.segments[1]?.kind, "item");
  assert.equal(blocks[0]?.segments[1]?.item.id, "error");
});

test("buildTranscriptDisplayBlocks passes through items without turnId", () => {
  const orphan = {
    id: "orphan",
    type: "lifecycle",
    title: "Wire parse error",
    text: "bad json",
    timestamp: baseTimestamp,
    acpSource: "acp_parse_error",
    channelId: "channel-1",
  };

  const blocks = buildTranscriptDisplayBlocks([orphan]);
  assert.equal(blocks.length, 1);
  assert.equal(blocks[0]?.kind, "single");
  assert.equal(blocks[0]?.item.id, "orphan");
});

test("buildTranscriptDisplayBlocks groups same-kind tool runs within a turn", () => {
  const items = [1, 2, 3].map((index) => ({
    id: `tool:${index}`,
    type: "tool",
    renderClass: "file-read",
    descriptor: {
      renderClass: "file-read",
      label: "Read file",
      preview: `file-${index}.ts`,
      groupKey: "read_file",
    },
    title: "read_file",
    toolName: "read_file",
    buzzToolName: null,
    status: "completed",
    args: { path: `file-${index}.ts` },
    result: "",
    isError: false,
    timestamp: "2026-06-18T00:00:00Z",
    startedAt: "2026-06-18T00:00:00Z",
    completedAt: "2026-06-18T00:00:01Z",
    turnId: "turn-1",
    sessionId: "sess-1",
    channelId: "chan-1",
  }));

  const [block] = buildTranscriptDisplayBlocks(items);

  assert.equal(block.kind, "turn");
  assert.equal(block.segments.length, 1);
  assert.equal(block.segments[0].kind, "summary");
  assert.equal(block.segments[0].summary.label, "Read 3 files");
});

test("buildTranscriptDisplayBlocks groups consecutive file edit tool runs", () => {
  const items = [1, 2].map((index) => ({
    id: `edit:${index}`,
    type: "tool",
    renderClass: "file-edit",
    descriptor: {
      renderClass: "file-edit",
      label: "Edited file",
      preview: `src/file-${index}.ts`,
      groupKey: "file-edit:str_replace",
    },
    title: "str_replace",
    toolName: "str_replace",
    buzzToolName: null,
    status: "completed",
    args: { path: `src/file-${index}.ts` },
    result: "",
    isError: false,
    timestamp: "2026-06-18T00:00:00Z",
    startedAt: "2026-06-18T00:00:00Z",
    completedAt: "2026-06-18T00:00:01Z",
    turnId: "turn-1",
    sessionId: "sess-1",
    channelId: "chan-1",
  }));

  const [block] = buildTranscriptDisplayBlocks(items);

  assert.equal(block.kind, "turn");
  assert.equal(block.segments.length, 1);
  assert.equal(block.segments[0].kind, "summary");
  assert.equal(block.segments[0].summary.label, "Edited 2 files");
  assert.equal(block.segments[0].summary.renderClass, "file-edit");
  assert.deepEqual(
    block.segments[0].summary.items.map((item) => item.id),
    ["edit:1", "edit:2"],
  );
});

test("buildTranscriptDisplayBlocks groups mixed consecutive eligible tool runs", () => {
  const [block] = buildTranscriptDisplayBlocks([
    mkTool("read-1", "Read file", "file-read", "read_file"),
    mkTool("shell-1", "Ran command", "shell", "shell:command"),
    mkTool("read-2", "Read file", "file-read", "read_file"),
    mkTool("read-3", "Read file", "file-read", "read_file"),
  ]);

  assert.equal(block.kind, "turn");
  assert.equal(block.segments.length, 1);
  assert.equal(block.segments[0].kind, "summary");
  assert.equal(block.segments[0].summary.variant, "mixed");
  assert.equal(block.segments[0].summary.label, "Ran 4 tool calls");
  assert.deepEqual(
    block.segments[0].summary.items.map((item) => item.id),
    ["read-1", "shell-1", "read-2", "read-3"],
  );
});

test("buildTranscriptDisplayBlocks groups tool bursts at threshold 2", () => {
  const [block] = buildTranscriptDisplayBlocks([
    mkTool("read-1", "Read file", "file-read", "read_file"),
    mkTool("shell-1", "Ran command", "shell", "shell:command"),
  ]);

  assert.equal(block.kind, "turn");
  assert.equal(block.segments.length, 1);
  assert.equal(block.segments[0].kind, "summary");
  assert.equal(block.segments[0].summary.variant, "mixed");
  assert.equal(block.segments[0].summary.label, "Ran 2 tool calls");
});

test("buildTranscriptDisplayBlocks keeps a lone eligible tool row expanded", () => {
  const [block] = buildTranscriptDisplayBlocks([
    mkTool("read-1", "Read file", "file-read", "read_file"),
  ]);

  assert.equal(block.kind, "turn");
  assert.deepEqual(
    block.segments.map((segment) => segment.kind),
    ["item"],
  );
});

test("buildTranscriptDisplayBlocks nests same-kind summaries inside tool bursts", () => {
  const [block] = buildTranscriptDisplayBlocks([
    mkTool("read-1", "Read file", "file-read", "read_file"),
    mkTool("read-2", "Read file", "file-read", "read_file"),
    mkTool("read-3", "Read file", "file-read", "read_file"),
    mkTool("shell-1", "Ran command", "shell", "shell:command"),
    mkTool("skill-1", "Read skill", "skill-read", "skill:load"),
  ]);

  assert.equal(block.kind, "turn");
  assert.equal(block.segments.length, 1);
  assert.equal(block.segments[0].kind, "summary");
  assert.equal(block.segments[0].summary.variant, "mixed");
  assert.equal(block.segments[0].summary.label, "Ran 5 tool calls");
  // Mixed summaries are the only visible burst summary; nested same-kind
  // summaries flatten back to leaf rows to avoid redundant rows such as
  // "Ran 16 tool calls" → "Ran 12 commands".
  assert.deepEqual(
    block.segments[0].summary.segments.map((child) =>
      child.kind === "item" ? child.item.id : child.summary.label,
    ),
    ["read-1", "read-2", "read-3", "shell-1", "skill-1"],
  );
  // Flat leaf items preserve original order.
  assert.deepEqual(
    block.segments[0].summary.items.map((item) => item.id),
    ["read-1", "read-2", "read-3", "shell-1", "skill-1"],
  );
});

test("buildTranscriptDisplayBlocks collapses alternating search/read bursts into one summary", () => {
  const [block] = buildTranscriptDisplayBlocks([
    mkTool("shell-1", "Ran command", "shell", "shell:command"),
    mkTool("read-1", "Read file", "file-read", "read_file"),
    mkTool("read-2", "Read file", "file-read", "read_file"),
    mkTool("read-3", "Read file", "file-read", "read_file"),
    mkTool("shell-2", "Ran command", "shell", "shell:command"),
    mkTool("read-4", "Read file", "file-read", "read_file"),
    mkTool("read-5", "Read file", "file-read", "read_file"),
    mkTool("read-6", "Read file", "file-read", "read_file"),
  ]);

  assert.equal(block.kind, "turn");
  assert.equal(block.segments.length, 1);
  assert.equal(block.segments[0].kind, "summary");
  assert.equal(block.segments[0].summary.variant, "mixed");
  assert.equal(block.segments[0].summary.label, "Ran 8 tool calls");
  assert.deepEqual(
    block.segments[0].summary.segments.map((child) =>
      child.kind === "summary" ? child.summary.label : child.item.id,
    ),
    [
      "shell-1",
      "read-1",
      "read-2",
      "read-3",
      "shell-2",
      "read-4",
      "read-5",
      "read-6",
    ],
  );
});

test("buildTranscriptDisplayBlocks keeps messages out of mixed tool runs", () => {
  const [block] = buildTranscriptDisplayBlocks([
    mkTool("read-1", "Read file", "file-read", "read_file"),
    mkTool("shell-1", "Ran command", "shell", "shell:command"),
    assistantMessage("assistant", "Here is what I found.", "turn-1"),
    mkTool("read-2", "Read file", "file-read", "read_file"),
    mkTool("shell-2", "Ran command", "shell", "shell:command"),
  ]);

  assert.equal(block.kind, "turn");
  assert.deepEqual(
    block.segments.map((segment) => segment.kind),
    ["summary", "item", "summary"],
  );
  assert.equal(block.segments[1].item.id, "assistant");
  assert.deepEqual(
    block.segments[0].summary.items.map((item) => item.id),
    ["read-1", "shell-1"],
  );
  assert.deepEqual(
    block.segments[2].summary.items.map((item) => item.id),
    ["read-2", "shell-2"],
  );
});

test("buildTranscriptDisplayBlocks breaks failed tools out of mixed tool runs", () => {
  const failed = {
    ...mkTool("shell-fail", "Ran command failed", "error", "shell:command"),
    isError: true,
  };

  const [block] = buildTranscriptDisplayBlocks([
    mkTool("read-1", "Read file", "file-read", "read_file"),
    mkTool("shell-1", "Ran command", "shell", "shell:command"),
    mkTool("skill-1", "Read skill", "skill-read", "skill:load"),
    failed,
    mkTool("read-2", "Read file", "file-read", "read_file"),
    mkTool("shell-2", "Ran command", "shell", "shell:command"),
    mkTool("image-1", "Viewed image", "image", "view_image"),
  ]);

  assert.equal(block.kind, "turn");
  assert.deepEqual(
    block.segments.map((segment) => segment.kind),
    ["summary", "item", "summary"],
  );
  assert.equal(block.segments[0].summary.variant, "mixed");
  assert.equal(block.segments[0].summary.label, "Ran 3 tool calls");
  assert.equal(block.segments[1].item.id, "shell-fail");
  assert.equal(block.segments[2].summary.variant, "mixed");
  assert.deepEqual(
    block.segments[2].summary.items.map((item) => item.id),
    ["read-2", "shell-2", "image-1"],
  );
});

test("flattenDisplayBlocks preserves child order through mixed summaries", () => {
  const blocks = buildTranscriptDisplayBlocks([
    mkTool("read-1", "Read file", "file-read", "read_file"),
    mkTool("shell-1", "Ran command", "shell", "shell:command"),
    mkTool("edit-1", "Edited file", "file-edit", "file-edit:str_replace"),
  ]);

  assert.deepEqual(
    flattenDisplayBlocks(blocks).map((item) => item.id),
    ["read-1", "shell-1", "edit-1"],
  );
});

test("buildTranscriptDisplayBlocks never same-kind groups failed tools", () => {
  const mkFailed = (id) => ({
    ...mkTool(id, "Ran command failed", "error", "shell:command"),
    isError: true,
  });

  const [block] = buildTranscriptDisplayBlocks([
    mkFailed("fail-1"),
    mkFailed("fail-2"),
    mkFailed("fail-3"),
  ]);

  assert.equal(block.kind, "turn");
  assert.deepEqual(
    block.segments.map((segment) => segment.kind),
    ["item", "item", "item"],
  );
});

test("buildTranscriptDisplayBlocks never same-kind groups status tool rows", () => {
  const [block] = buildTranscriptDisplayBlocks([
    mkTool("status-1", "Context compacted", "status", "status:post-compact"),
    mkTool("status-2", "Context compacted", "status", "status:post-compact"),
    mkTool("status-3", "Context compacted", "status", "status:post-compact"),
  ]);

  assert.equal(block.kind, "turn");
  assert.deepEqual(
    block.segments.map((segment) => segment.kind),
    ["item", "item", "item"],
  );
});

test("buildTranscriptDisplayBlocks never same-kind groups suppressed tool rows", () => {
  const [block] = buildTranscriptDisplayBlocks([
    mkTool("stop-1", "Checked todos", "suppressed", "suppressed:stop-hook"),
    mkTool("stop-2", "Checked todos", "suppressed", "suppressed:stop-hook"),
    mkTool("stop-3", "Checked todos", "suppressed", "suppressed:stop-hook"),
  ]);

  assert.equal(block.kind, "turn");
  assert.deepEqual(
    block.segments.map((segment) => segment.kind),
    ["item", "item", "item"],
  );
});

test("buildTranscriptDisplayBlocks breaks same-kind runs on an ineligible row", () => {
  const failed = {
    ...mkTool("fail-1", "Read file failed", "error", "read_file"),
    isError: true,
  };

  const [block] = buildTranscriptDisplayBlocks([
    mkTool("read-1", "Read file", "file-read", "read_file"),
    mkTool("read-2", "Read file", "file-read", "read_file"),
    failed,
    mkTool("read-3", "Read file", "file-read", "read_file"),
    mkTool("read-4", "Read file", "file-read", "read_file"),
  ]);

  assert.equal(block.kind, "turn");
  assert.deepEqual(
    block.segments.map((segment) => segment.kind),
    ["summary", "item", "summary"],
  );
  assert.equal(block.segments[1].item.id, "fail-1");
  assert.deepEqual(
    block.segments[0].summary.items.map((item) => item.id),
    ["read-1", "read-2"],
  );
  assert.deepEqual(
    block.segments[2].summary.items.map((item) => item.id),
    ["read-3", "read-4"],
  );
});

test("buildTranscriptDisplayBlocks bundles steer message with steer context behind the prompt segment", () => {
  const steerMessage = {
    id: "steer:chan-1:turn-1",
    type: "message",
    role: "user",
    title: "Buzz event",
    text: "@Bart new steer instruction",
    timestamp: baseTimestamp,
    acpSource: "session/steer:user",
    turnId: "turn-1",
    sessionId: "sess-1",
    channelId: "chan-1",
  };
  const steerContext = {
    id: "steer-context:chan-1:turn-1",
    type: "metadata",
    title: "Prompt context",
    sections: [{ title: "Thread history", body: "prior messages" }],
    timestamp: baseTimestamp,
    acpSource: "session/steer:context",
    turnId: "turn-1",
    sessionId: "sess-1",
    channelId: "chan-1",
  };

  const [block] = buildTranscriptDisplayBlocks([
    assistantMessage("assistant", "Working on it.", "turn-1"),
    steerMessage,
    steerContext,
    toolCall("tool", "turn-1"),
  ]);

  assert.equal(block.kind, "turn");
  assert.deepEqual(
    block.segments.map((segment) => segment.kind),
    ["item", "prompt", "item"],
  );
  const steerSegment = block.segments[1];
  assert.equal(steerSegment.user.id, "steer:chan-1:turn-1");
  assert.equal(steerSegment.context?.id, "steer-context:chan-1:turn-1");
  assert.deepEqual(steerSegment.setup, []);
  // No standalone "Prompt context" metadata row leaks into the feed.
  assert.ok(
    !block.segments.some(
      (segment) => segment.kind === "item" && segment.item.type === "metadata",
    ),
  );
});

test("buildTranscriptDisplayBlocks keeps orphan steer context visible when no steer message exists", () => {
  const steerContext = {
    id: "steer-context:chan-1:turn-1",
    type: "metadata",
    title: "Prompt context",
    sections: [{ title: "Thread history", body: "prior messages" }],
    timestamp: baseTimestamp,
    acpSource: "session/steer:context",
    turnId: "turn-1",
    sessionId: "sess-1",
    channelId: "chan-1",
  };

  const [block] = buildTranscriptDisplayBlocks([
    steerContext,
    toolCall("tool", "turn-1"),
  ]);

  assert.equal(block.kind, "turn");
  const flattened = flattenDisplayBlocks([block]).map((item) => item.id);
  assert.ok(flattened.includes("steer-context:chan-1:turn-1"));
});

function mkTool(id, label, renderClass = "generic", groupKey = label) {
  return {
    id,
    type: "tool",
    renderClass,
    descriptor: {
      renderClass,
      label,
      preview: id,
      source: "harness",
      groupKey,
    },
    title: label,
    toolName: label,
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp: "2026-06-18T00:00:00Z",
    startedAt: "2026-06-18T00:00:00Z",
    completedAt: "2026-06-18T00:00:01Z",
    turnId: "turn-1",
    sessionId: "sess-1",
    channelId: "chan-1",
  };
}

// ── Session-run splitting and session-boundary blocks ──────────────────────────

/**
 * Build a minimal tool-call item stamped with a specific session.
 */
function sessionItem(id, sessionId, ts = "2026-07-08T00:00:00.000Z") {
  return {
    id,
    type: "tool",
    renderClass: "generic",
    descriptor: {
      renderClass: "generic",
      label: id,
      preview: id,
      source: "harness",
      groupKey: id,
    },
    title: id,
    toolName: id,
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp: ts,
    startedAt: ts,
    completedAt: ts,
    turnId: `turn-${id}`,
    sessionId,
    channelId: "chan-1",
  };
}

// ── Single session — no boundary injected ──────────────────────────────────────

test("buildTranscriptDisplayBlocks_singleSession_noBoundaryBlock", () => {
  const items = [sessionItem("a", "sess-1"), sessionItem("b", "sess-1")];
  const blocks = buildTranscriptDisplayBlocks(items);
  const boundaryBlocks = blocks.filter((b) => b.kind === "session-boundary");
  assert.equal(
    boundaryBlocks.length,
    0,
    "no session-boundary blocks for a single session",
  );
});

// ── Two sessions — one boundary between them ───────────────────────────────────

test("buildTranscriptDisplayBlocks_twoSessions_oneBoundaryBetween", () => {
  // items ordered oldest-first: sess-1 then sess-2
  const items = [
    sessionItem("a", "sess-1", "2026-07-08T00:00:01.000Z"),
    sessionItem("b", "sess-2", "2026-07-08T00:00:02.000Z"),
  ];
  const blocks = buildTranscriptDisplayBlocks(items);
  const boundaryBlocks = blocks.filter((b) => b.kind === "session-boundary");
  assert.equal(
    boundaryBlocks.length,
    1,
    "exactly one boundary for two sessions",
  );
  // The boundary is inserted BEFORE the newer run (sess-2).
  const boundaryIndex = blocks.indexOf(boundaryBlocks[0]);
  const prevBlock = blocks[boundaryIndex - 1];
  const nextBlock = blocks[boundaryIndex + 1];
  // Previous content block belongs to sess-1 items, next to sess-2.
  const flatPrev = flattenDisplayBlocks([prevBlock]).map((i) => i.id);
  const flatNext = flattenDisplayBlocks([nextBlock]).map((i) => i.id);
  assert.ok(flatPrev.includes("a"), "content before boundary is sess-1");
  assert.ok(flatNext.includes("b"), "content after boundary is sess-2");
});

// ── Newest session labeled correctly relative to latestLiveSessionId ──────────

test("buildTranscriptDisplayBlocks_newestMatchesLive_labelStateCurrent", () => {
  const items = [
    sessionItem("old", "sess-1", "2026-07-08T00:00:01.000Z"),
    sessionItem("new", "sess-2", "2026-07-08T00:00:02.000Z"),
  ];
  const blocks = buildTranscriptDisplayBlocks(items, "sess-2");
  const boundary = blocks.find((b) => b.kind === "session-boundary");
  assert.ok(boundary, "boundary present");
  assert.equal(
    boundary.labelState,
    "current",
    "labelState=current when newest session matches live id",
  );
  assert.equal(boundary.sessionId, "sess-2", "boundary sessionId is sess-2");
});

test("buildTranscriptDisplayBlocks_newestNoLive_labelStateMostRecent", () => {
  const items = [
    sessionItem("old", "sess-1", "2026-07-08T00:00:01.000Z"),
    sessionItem("new", "sess-2", "2026-07-08T00:00:02.000Z"),
  ];
  // latestLiveSessionId is null → newest session is "most-recent"
  const blocksNoLive = buildTranscriptDisplayBlocks(items, null);
  const boundaryNoLive = blocksNoLive.find(
    (b) => b.kind === "session-boundary",
  );
  assert.equal(
    boundaryNoLive.labelState,
    "most-recent",
    "labelState=most-recent when no live id (archived-only view)",
  );

  // latestLiveSessionId is a DIFFERENT session → newest is still "most-recent"
  const blocksDiffLive = buildTranscriptDisplayBlocks(items, "sess-other");
  const boundaryDiff = blocksDiffLive.find(
    (b) => b.kind === "session-boundary",
  );
  assert.equal(
    boundaryDiff.labelState,
    "most-recent",
    "labelState=most-recent when live id differs from newest visible",
  );
});

// ── Three sessions — two boundaries ────────────────────────────────────────────

test("buildTranscriptDisplayBlocks_threeSessions_twoBoundaries", () => {
  const items = [
    sessionItem("a", "sess-1", "2026-07-08T00:00:01.000Z"),
    sessionItem("b", "sess-2", "2026-07-08T00:00:02.000Z"),
    sessionItem("c", "sess-3", "2026-07-08T00:00:03.000Z"),
  ];
  const blocks = buildTranscriptDisplayBlocks(items);
  const boundaryBlocks = blocks.filter((b) => b.kind === "session-boundary");
  assert.equal(boundaryBlocks.length, 2, "two boundaries for three sessions");
  // With no live id: newest (sess-3) boundary = "most-recent"; older = "earlier".
  const newestBoundary = boundaryBlocks[boundaryBlocks.length - 1];
  const olderBoundary = boundaryBlocks[0];
  assert.equal(
    newestBoundary.labelState,
    "most-recent",
    "newest boundary is most-recent when latestLiveSessionId is null",
  );
  assert.equal(
    olderBoundary.labelState,
    "earlier",
    "older boundary is earlier when latestLiveSessionId is null",
  );
});

// ── Null sessionId items stay in the current run ───────────────────────────────

test("buildTranscriptDisplayBlocks_nullSessionId_staysInCurrentRun", () => {
  // An item with null sessionId should not start a new run.
  const items = [
    sessionItem("a", "sess-1", "2026-07-08T00:00:01.000Z"),
    // null sessionId — stays in sess-1 run
    { ...sessionItem("b", null, "2026-07-08T00:00:02.000Z"), sessionId: null },
    sessionItem("c", "sess-1", "2026-07-08T00:00:03.000Z"),
  ];
  const blocks = buildTranscriptDisplayBlocks(items);
  const boundaryBlocks = blocks.filter((b) => b.kind === "session-boundary");
  assert.equal(
    boundaryBlocks.length,
    0,
    "no boundary when null-sessionId items are present within a single session",
  );
});

// ── flattenDisplayBlocks skips session-boundary blocks ────────────────────────

test("flattenDisplayBlocks_skipsSessionBoundaryBlocks", () => {
  const items = [
    sessionItem("a", "sess-1", "2026-07-08T00:00:01.000Z"),
    sessionItem("b", "sess-2", "2026-07-08T00:00:02.000Z"),
  ];
  const blocks = buildTranscriptDisplayBlocks(items);
  assert.ok(
    blocks.some((b) => b.kind === "session-boundary"),
    "test setup: boundary must be present",
  );
  const flat = flattenDisplayBlocks(blocks);
  const ids = flat.map((i) => i.id);
  assert.ok(ids.includes("a"), "item a is in flattened output");
  assert.ok(ids.includes("b"), "item b is in flattened output");
  assert.equal(
    flat.filter((i) => i.kind === "session-boundary").length,
    0,
    "session-boundary items are excluded from flatten",
  );
});

// ── isObserverEventAfter — latest-live ordering ──────────────────────────────

test("isObserverEventAfter returns true when candidate has later timestamp", () => {
  const stored = { timestamp: "2026-07-08T00:00:01.000Z", seq: 5 };
  const candidate = { timestamp: "2026-07-08T00:00:02.000Z", seq: 1 };
  assert.ok(isObserverEventAfter(candidate, stored));
});

test("isObserverEventAfter returns false when candidate has earlier timestamp", () => {
  const stored = { timestamp: "2026-07-08T00:00:02.000Z", seq: 5 };
  const candidate = { timestamp: "2026-07-08T00:00:01.000Z", seq: 10 };
  assert.ok(!isObserverEventAfter(candidate, stored));
});

test("isObserverEventAfter returns true for same timestamp, higher seq — session B advances over session A", () => {
  // This is the tiebreak case: timestamp equal, seq tiebreak must mirror
  // compareObserverEvents so latest-live never drifts from transcript order.
  const stored = { timestamp: "2026-07-08T00:00:01.000Z", seq: 3 };
  const candidate = { timestamp: "2026-07-08T00:00:01.000Z", seq: 7 };
  assert.ok(isObserverEventAfter(candidate, stored));
});

test("isObserverEventAfter returns false for same timestamp, same seq", () => {
  const stored = { timestamp: "2026-07-08T00:00:01.000Z", seq: 3 };
  const candidate = { timestamp: "2026-07-08T00:00:01.000Z", seq: 3 };
  assert.ok(!isObserverEventAfter(candidate, stored));
});

test("isObserverEventAfter returns false for same timestamp, lower seq", () => {
  const stored = { timestamp: "2026-07-08T00:00:01.000Z", seq: 7 };
  const candidate = { timestamp: "2026-07-08T00:00:01.000Z", seq: 3 };
  assert.ok(!isObserverEventAfter(candidate, stored));
});

// ── Pre-resolution null-session deferral (regression for first-turn bundle) ───

/**
 * Builds the exact wire sequence the harness emits on the first turn:
 *   turn_started(null) → session/new(null) → session_resolved(sess) → session/prompt(sess)
 *
 * After processTranscriptEvent the TranscriptItems produced are:
 *   - lifecycle turn_started: sessionId=null, turnId="turn-001"
 *   - metadata session/new (system prompt): sessionId=null, turnId=null
 *   - lifecycle session_resolved: sessionId="session-001", turnId="turn-001"
 *   - message session/prompt:user: sessionId="session-001", turnId="turn-001"
 */
function firstTurnSequence() {
  const ts = "2026-07-08T10:00:00.000Z";
  return [
    // turn_started: sessionId null, turnId present
    {
      id: "turn-started",
      type: "lifecycle",
      renderClass: "lifecycle",
      title: "Turn started",
      text: "",
      timestamp: ts,
      acpSource: "turn_started",
      turnId: "turn-001",
      sessionId: null,
      channelId: "chan-1",
    },
    // session/new system prompt: sessionId null, turnId null (processTranscriptEvent forces turnId null)
    {
      id: "system-prompt:chan-1",
      type: "metadata",
      renderClass: "raw-rail",
      title: "System prompt",
      sections: [
        { title: "Base", body: "You are a helpful AI assistant." },
        { title: "System", body: "You are Observer Agent." },
      ],
      timestamp: ts,
      acpSource: "session/new",
      turnId: null,
      sessionId: null,
      channelId: "chan-1",
    },
    // session_resolved: first item with a non-null sessionId
    {
      id: "session-resolved",
      type: "lifecycle",
      renderClass: "lifecycle",
      title: "Session ready",
      text: "",
      timestamp: ts,
      acpSource: "session_resolved",
      turnId: "turn-001",
      sessionId: "session-001",
      channelId: "chan-1",
    },
    // session/prompt:user — the user message bubble
    {
      id: "user-prompt",
      type: "message",
      role: "user",
      title: "Buzz event",
      text: "@Observer Agent help me debug this",
      timestamp: ts,
      acpSource: "session/prompt:user",
      turnId: "turn-001",
      sessionId: "session-001",
      channelId: "chan-1",
    },
  ];
}

test("buildTranscriptDisplayBlocks_firstTurnSequence_standaloneSystemPrompt", () => {
  // session/new is session-scoped, not turn-scoped. After the consolidation,
  // the grouper emits it as a standalone "System prompt" single block that
  // appears BEFORE the first user-prompt turn — never inside the prompt bundle.
  const blocks = buildTranscriptDisplayBlocks(firstTurnSequence());

  // (a) Exactly one standalone "System prompt" single block.
  const systemPromptSingles = blocks.filter(
    (b) => b.kind === "single" && b.item.acpSource === "session/new",
  );
  assert.equal(
    systemPromptSingles.length,
    1,
    "system prompt must appear as exactly one standalone single block",
  );

  // (b) No session-boundary blocks — this is a single-session transcript.
  const boundaryBlocks = blocks.filter((b) => b.kind === "session-boundary");
  assert.equal(
    boundaryBlocks.length,
    0,
    "must be zero session-boundary blocks for a first-turn single-session sequence",
  );

  // (c) System prompt is NOT inside any turn group (not in prompt bundle).
  const turnBlocks = blocks.filter((b) => b.kind === "turn");
  assert.ok(turnBlocks.length > 0, "at least one turn block must exist");
  const allTurnItems = flattenDisplayBlocks(turnBlocks);
  const systemPromptInTurn = allTurnItems.some(
    (item) => item.acpSource === "session/new",
  );
  assert.ok(
    !systemPromptInTurn,
    "system prompt item must NOT be present inside a turn block",
  );

  // (d) Standalone system-prompt block appears BEFORE the first turn block.
  const systemPromptIdx = blocks.indexOf(systemPromptSingles[0]);
  const firstTurnIdx = blocks.findIndex((b) => b.kind === "turn");
  assert.ok(
    firstTurnIdx !== -1,
    "there must be at least one turn block after the system prompt",
  );
  assert.ok(
    systemPromptIdx < firstTurnIdx,
    `system-prompt single (${systemPromptIdx}) must precede first turn block (${firstTurnIdx})`,
  );
});

test("buildTranscriptDisplayBlocks_genuineSecondSession_boundaryPreserved", () => {
  // Regression guard: the deferral fix must not over-collapse two genuinely
  // distinct sessions. A second session_resolved with a different sessionId
  // still gets its own run and a session-boundary block.
  const ts2 = "2026-07-08T11:00:00.000Z";
  const items = [
    // First session (may have pre-resolution preamble)
    ...firstTurnSequence(),
    // Second session — starts fresh with its own turn_started (no pre-null preamble here)
    {
      id: "turn-started-2",
      type: "lifecycle",
      renderClass: "lifecycle",
      title: "Turn started",
      text: "",
      timestamp: ts2,
      acpSource: "turn_started",
      turnId: "turn-002",
      sessionId: "session-002",
      channelId: "chan-1",
    },
    {
      id: "user-prompt-2",
      type: "message",
      role: "user",
      title: "Buzz event",
      text: "second session message",
      timestamp: ts2,
      acpSource: "session/prompt:user",
      turnId: "turn-002",
      sessionId: "session-002",
      channelId: "chan-1",
    },
  ];

  const blocks = buildTranscriptDisplayBlocks(items);
  const boundaryBlocks = blocks.filter((b) => b.kind === "session-boundary");
  assert.equal(
    boundaryBlocks.length,
    1,
    "exactly one session-boundary block between two distinct sessions",
  );
  assert.equal(
    boundaryBlocks[0].sessionId,
    "session-002",
    "boundary is labeled with the newer session id",
  );
});

// ── Duplicate-key guard: non-contiguous runs of the same sessionId ────────────

test("buildTranscriptDisplayBlocks_nonContiguousRunsSameSession_distinctBoundaryKeys", () => {
  // Scenario: sess-B frames, then sess-A frames, then sess-C frames, then
  // sess-A frames re-resolve (e.g. same agent session re-observed in a new
  // live subscription). splitIntoSessionRuns treats each session-id change as
  // a new run, so sess-A appears twice — once at runIndex=1 and once at
  // runIndex=3 — yielding two session-boundary blocks both with
  // sessionId="sess-A".
  //
  // The React key for a session-boundary is
  //   `session-boundary:${sessionId}:${runIndex}`
  // Without the runIndex tiebreaker the two sess-A boundaries share the same
  // key, causing React to silently duplicate or omit children. This test
  // asserts the runIndex tiebreaker keeps the keys distinct.
  const items = [
    sessionItem("b-item", "sess-B", "2026-07-08T00:00:01.000Z"),
    sessionItem("a-item", "sess-A", "2026-07-08T00:00:02.000Z"),
    sessionItem("c-item", "sess-C", "2026-07-08T00:00:03.000Z"),
    sessionItem("a-item-2", "sess-A", "2026-07-08T00:00:04.000Z"),
  ];

  const blocks = buildTranscriptDisplayBlocks(items, "sess-A");
  const boundaryBlocks = blocks.filter((b) => b.kind === "session-boundary");

  // Four distinct session runs → three boundaries (before sess-A, before
  // sess-C, and before the re-occurring sess-A).
  assert.equal(
    boundaryBlocks.length,
    3,
    "three boundaries for four runs (sess-B, sess-A, sess-C, sess-A)",
  );

  // Two boundaries share sessionId="sess-A" (the collision case pre-fix).
  const sessABoundaries = boundaryBlocks.filter(
    (b) => b.sessionId === "sess-A",
  );
  assert.equal(
    sessABoundaries.length,
    2,
    "two boundary blocks both carry sessionId=sess-A (the collision the fix guards)",
  );

  // The runIndex values must be distinct across ALL boundary blocks so that
  // `session-boundary:${sessionId}:${runIndex}` keys are unique.
  const runIndexSet = new Set(boundaryBlocks.map((b) => b.runIndex));
  assert.equal(
    runIndexSet.size,
    boundaryBlocks.length,
    "every session-boundary block has a distinct runIndex",
  );

  // The React keys derived from each boundary must also be distinct.
  const keys = boundaryBlocks.map(
    (b) => `session-boundary:${b.sessionId}:${b.runIndex}`,
  );
  const keySet = new Set(keys);
  assert.equal(
    keySet.size,
    keys.length,
    "all React keys derived from session-boundary blocks are unique",
  );
});

// ── session/new run-anchor: restart scenario ──────────────────────────────────

/**
 * Build a system-prompt metadata item as the normalizer produces it on restart:
 * stale-stamped with the previous session's id (latestSessionId at emit time).
 */
function systemPromptItem(id, staleSessionId, ts = "2026-07-08T12:00:00.000Z") {
  return {
    id,
    type: "metadata",
    renderClass: "raw-rail",
    title: "System prompt",
    sections: [{ title: "Base", body: "You are a helpful assistant." }],
    timestamp: ts,
    acpSource: "session/new",
    turnId: null,
    sessionId: staleSessionId,
    channelId: "chan-1",
  };
}

test("buildTranscriptDisplayBlocks_restartScenario_systemPromptAfterBoundary", () => {
  // Restart wire sequence: toolA(sess-1) → session/new(sess-1 stale) → toolB(sess-2)
  // The session/new item is stale-stamped with sess-1 (the OLD session id).
  // After the fix it must sort into the sess-2 run — AFTER the session-boundary
  // block, not before it.
  const ts1 = "2026-07-08T12:00:00.000Z";
  const ts2 = "2026-07-08T12:01:00.000Z";
  const items = [
    // Prior session activity
    { ...sessionItem("toolA", "sess-1", ts1), turnId: "turn-1" },
    // session/new stale-stamped with OLD session id (the bug scenario)
    systemPromptItem("system-prompt", "sess-1", ts2),
    // New session activity
    { ...sessionItem("toolB", "sess-2", ts2), turnId: "turn-2" },
  ];

  const blocks = buildTranscriptDisplayBlocks(items, "sess-2");

  // (a) Exactly one session-boundary block must exist.
  const boundaryBlocks = blocks.filter((b) => b.kind === "session-boundary");
  assert.equal(
    boundaryBlocks.length,
    1,
    "exactly one session-boundary block for two sessions",
  );

  // (b) The session/new item must appear AFTER the boundary, not before it.
  const boundaryIndex = blocks.indexOf(boundaryBlocks[0]);
  const systemPromptBlockIndex = blocks.findIndex(
    (b) => b.kind === "single" && b.item.acpSource === "session/new",
  );
  assert.ok(
    systemPromptBlockIndex !== -1,
    "system-prompt item must be present in the output",
  );
  assert.ok(
    boundaryIndex < systemPromptBlockIndex,
    `boundary (index ${boundaryIndex}) must come before system-prompt (index ${systemPromptBlockIndex})`,
  );

  // (c) toolA must appear before the boundary; toolB after.
  const flatAll = flattenDisplayBlocks(blocks);
  const ids = flatAll.map((i) => i.id);
  assert.ok(ids.includes("toolA"), "toolA present in flattened output");
  assert.ok(ids.includes("toolB"), "toolB present in flattened output");
  const toolAIdx = blocks.findIndex((b) =>
    flattenDisplayBlocks([b]).some((i) => i.id === "toolA"),
  );
  const toolBIdx = blocks.findIndex((b) =>
    flattenDisplayBlocks([b]).some((i) => i.id === "toolB"),
  );
  assert.ok(
    toolAIdx < boundaryIndex,
    "toolA block must be before the boundary",
  );
  assert.ok(toolBIdx > boundaryIndex, "toolB block must be after the boundary");
});

test("buildTranscriptDisplayBlocks_firstEverSession_systemPromptInSingleRun", () => {
  // First-ever session: session/new arrives with sessionId null before any
  // session resolves — the preSessionBuffer path handles it, no boundary emitted.
  // This test guards against regressing the first-session behavior.
  const ts = "2026-07-08T10:00:00.000Z";
  const items = [
    // session/new with null sessionId (first ever, no stale-stamp)
    {
      id: "system-prompt",
      type: "metadata",
      renderClass: "raw-rail",
      title: "System prompt",
      sections: [{ title: "Base", body: "You are a helpful assistant." }],
      timestamp: ts,
      acpSource: "session/new",
      turnId: null,
      sessionId: null,
      channelId: "chan-1",
    },
    // session_resolved gives the first non-null sessionId
    {
      id: "session-resolved",
      type: "lifecycle",
      renderClass: "lifecycle",
      title: "Session ready",
      text: "",
      timestamp: ts,
      acpSource: "session_resolved",
      turnId: "turn-001",
      sessionId: "session-001",
      channelId: "chan-1",
    },
    // User prompt follows
    {
      id: "user-prompt",
      type: "message",
      role: "user",
      title: "Buzz event",
      text: "@Agent hello",
      timestamp: ts,
      acpSource: "session/prompt:user",
      turnId: "turn-001",
      sessionId: "session-001",
      channelId: "chan-1",
    },
  ];

  const blocks = buildTranscriptDisplayBlocks(items, "session-001");

  // No boundary — this is a single-session transcript.
  const boundaryBlocks = blocks.filter((b) => b.kind === "session-boundary");
  assert.equal(
    boundaryBlocks.length,
    0,
    "no session-boundary block for a first-ever single session",
  );

  // System prompt must appear somewhere in the output (as a standalone single).
  const flat = flattenDisplayBlocks(blocks);
  assert.ok(
    flat.some((i) => i.acpSource === "session/new"),
    "system-prompt item must be present in the single-session output",
  );
});

test("buildTranscriptDisplayBlocks_sessionNewNoFollowingSession_notDropped", () => {
  // session/new arrives after a resolved session but the stream ends before a
  // new session resolves (e.g. agent shutdown mid-restart). The item must not
  // be silently dropped — it should fall back into the current run.
  const ts1 = "2026-07-08T12:00:00.000Z";
  const ts2 = "2026-07-08T12:01:00.000Z";
  const items = [
    { ...sessionItem("toolA", "sess-1", ts1), turnId: "turn-1" },
    // session/new stale-stamped; stream ends here — no new session resolves.
    systemPromptItem("system-prompt", "sess-1", ts2),
  ];

  const blocks = buildTranscriptDisplayBlocks(items);

  // No boundary — still a single session (no new session resolved).
  const boundaryBlocks = blocks.filter((b) => b.kind === "session-boundary");
  assert.equal(
    boundaryBlocks.length,
    0,
    "no boundary when session/new has no following new session",
  );

  // system-prompt must not be dropped.
  const flat = flattenDisplayBlocks(blocks);
  assert.ok(
    flat.some((i) => i.id === "system-prompt"),
    "system-prompt must not be dropped when no new session follows",
  );
  // toolA must also be present.
  assert.ok(
    flat.some((i) => i.id === "toolA"),
    "toolA must be present when session/new has no following new session",
  );
});

// --- no-user-prompt ordering: system-prompt leads turns in the same run ─────

test("splitIntoSessionRuns: system-prompt renders before turn blocks when no user-prompt follows", () => {
  // Covers buildBlocksForRun's no-user-prompt branch: when session/new is
  // followed by a tool turn but no session/prompt:user, the system-prompt must
  // appear as a standalone block BEFORE the turn (boundary → prompt → activity),
  // not appended after all turns (the old behaviour).
  const ts1 = "2026-07-08T12:00:00.000Z";
  const ts2 = "2026-07-08T12:01:00.000Z";
  const ts3 = "2026-07-08T12:02:00.000Z";
  const items = [
    // A preceding tool item establishes run sess-1 so a boundary is emitted.
    { ...sessionItem("toolA", "sess-1", ts1), turnId: "turn-1" },
    // system/new repositioned to tail (stale sess-1 stamp) — represents the
    // restart marker after the normalizer's reposition-on-refire fix.
    systemPromptItem("system-prompt", "sess-1", ts2),
    // New run's tool item (no user prompt in turn-2).
    { ...sessionItem("toolB", "sess-2", ts3), turnId: "turn-2" },
  ];

  const blocks = buildTranscriptDisplayBlocks(items);

  // One boundary between the two sessions.
  const boundaryBlocks = blocks.filter((b) => b.kind === "session-boundary");
  assert.equal(boundaryBlocks.length, 1, "exactly one boundary");

  const boundaryIdx = blocks.indexOf(boundaryBlocks[0]);

  const systemPromptIdx = blocks.findIndex(
    (b) => b.kind === "single" && b.item?.id === "system-prompt",
  );
  assert.ok(systemPromptIdx !== -1, "system-prompt block must be present");

  const toolBIdx = blocks.findIndex((b) =>
    flattenDisplayBlocks([b]).some((i) => i.id === "toolB"),
  );
  assert.ok(toolBIdx !== -1, "toolB block must be present");

  // Required order: boundary → system-prompt → toolB activity.
  assert.ok(
    boundaryIdx < systemPromptIdx,
    `boundary (${boundaryIdx}) must precede system-prompt (${systemPromptIdx})`,
  );
  assert.ok(
    systemPromptIdx < toolBIdx,
    `system-prompt (${systemPromptIdx}) must precede toolB activity (${toolBIdx})`,
  );

  // toolA (sess-1) must appear before the boundary.
  const toolAIdx = blocks.findIndex((b) =>
    flattenDisplayBlocks([b]).some((i) => i.id === "toolA"),
  );
  assert.ok(toolAIdx !== -1, "toolA block must be present");
  assert.ok(
    toolAIdx < boundaryIdx,
    `toolA (${toolAIdx}) must be before boundary (${boundaryIdx})`,
  );
});

// ── Session-boundary firstItemId key stability (regression) ───────────────────
//
// Previously getDisplayBlockKey keyed boundaries as
// `session-boundary:${sessionId}:${runIndex}`. `runIndex` is the run's
// position in the ordered array — it SHIFTS when older sessions are prepended
// (archive page load), causing React to remount unchanged boundaries and churn
// the virtual list.
//
// The fix replaces `runIndex` with `firstItemId` — the id of the first
// TranscriptItem in the run — which is invariant across prepend.

test("buildTranscriptDisplayBlocks_sessionBoundary_emitsFirstItemId", () => {
  // Two sessions: sess-1 (older) then sess-2 (newer).
  const items = [
    sessionItem("a", "sess-1", "2026-07-08T00:00:01.000Z"),
    sessionItem("b", "sess-2", "2026-07-08T00:00:02.000Z"),
    sessionItem("c", "sess-2", "2026-07-08T00:00:03.000Z"),
  ];
  const blocks = buildTranscriptDisplayBlocks(items, null);
  const boundary = blocks.find((b) => b.kind === "session-boundary");
  assert.ok(boundary, "boundary must be present between two sessions");
  assert.equal(
    boundary.sessionId,
    "sess-2",
    "boundary labels the newer session",
  );
  // firstItemId must equal the id of the first item in sess-2's run.
  assert.ok(boundary.firstItemId, "boundary must carry firstItemId");
  // "b" is the first item in sess-2's run (from sessionItem("b", "sess-2")).
  assert.equal(
    boundary.firstItemId,
    "b",
    "firstItemId must equal the id of the first item in the following session run",
  );
});

test("buildTranscriptDisplayBlocks_sessionBoundary_keyStableAcrossPrepend", () => {
  // Before: sess-1 then sess-2.
  const before = [
    sessionItem("a", "sess-1", "2026-07-08T00:00:02.000Z"),
    sessionItem("b", "sess-2", "2026-07-08T00:00:03.000Z"),
  ];
  const blocksBefore = buildTranscriptDisplayBlocks(before, null);
  const boundaryBefore = blocksBefore.find(
    (b) => b.kind === "session-boundary" && b.sessionId === "sess-2",
  );
  assert.ok(boundaryBefore, "must have sess-2 boundary before prepend");
  const keyBefore = `session-boundary:${boundaryBefore.sessionId}:${boundaryBefore.firstItemId}`;

  // After: prepend an older sess-0 before sess-1. Now [sess-0, sess-1, sess-2].
  const after = [
    sessionItem("z", "sess-0", "2026-07-08T00:00:01.000Z"), // oldest — prepended
    sessionItem("a", "sess-1", "2026-07-08T00:00:02.000Z"),
    sessionItem("b", "sess-2", "2026-07-08T00:00:03.000Z"),
  ];
  const blocksAfter = buildTranscriptDisplayBlocks(after, null);
  const boundaryAfter = blocksAfter.find(
    (b) => b.kind === "session-boundary" && b.sessionId === "sess-2",
  );
  assert.ok(boundaryAfter, "must still have sess-2 boundary after prepend");
  const keyAfter = `session-boundary:${boundaryAfter.sessionId}:${boundaryAfter.firstItemId}`;

  // firstItemId-based key must be identical before and after prepend.
  assert.equal(
    keyBefore,
    keyAfter,
    "session-boundary React key must not change when an older session is prepended",
  );

  // Sanity check: runIndex DID shift (from 1 to 2) confirming the old key would have changed.
  assert.equal(boundaryBefore.runIndex, 1, "runIndex before prepend must be 1");
  assert.equal(
    boundaryAfter.runIndex,
    2,
    "runIndex after prepend must be 2, confirming it is unstable",
  );
});

// ── Multiple pre-resolution session/new markers — both survive ────────────────

test("buildTranscriptDisplayBlocks_twoPreResolutionSessionNewMarkers_bothSurviveBeforeTurn", () => {
  // Restart loop: two session/new items arrive after sess-1 resolves but before
  // sess-2 resolves (e.g. a rapid double-restart). A null-session frame arrives
  // between them (models a lifecycle event with no session yet). All three must
  // appear in the sess-2 run — neither marker nor the interleaved null-session
  // frame may be silently dropped. This is the documented
  // "marker(s) plus trailing null-session items" contract.
  //
  // Wire order:
  //   toolA(sess-1) → sp-a(sess-1 stale) → null-frame(null) → sp-b(sess-1 stale) → toolB(sess-2)
  const ts1 = "2026-07-08T12:00:00.000Z";
  const ts2 = "2026-07-08T12:01:00.000Z";
  const ts3 = "2026-07-08T12:01:15.000Z";
  const ts4 = "2026-07-08T12:01:30.000Z";
  const ts5 = "2026-07-08T12:02:00.000Z";

  // A null-session lifecycle frame (models turn_started or similar arriving
  // between two session/new firings during a rapid double-restart).
  const nullFrame = {
    id: "null-frame",
    type: "lifecycle",
    renderClass: "lifecycle",
    title: "Session starting",
    text: "",
    timestamp: ts3,
    acpSource: "turn_started",
    turnId: null,
    sessionId: null,
    channelId: "chan-1",
  };

  const items = [
    { ...sessionItem("toolA", "sess-1", ts1), turnId: "turn-1" },
    systemPromptItem("sp-a", "sess-1", ts2),
    nullFrame,
    systemPromptItem("sp-b", "sess-1", ts4),
    { ...sessionItem("toolB", "sess-2", ts5), turnId: "turn-2" },
  ];

  const blocks = buildTranscriptDisplayBlocks(items, "sess-2");

  // Exactly one boundary between the two sessions.
  const boundaryBlocks = blocks.filter((b) => b.kind === "session-boundary");
  assert.equal(
    boundaryBlocks.length,
    1,
    "exactly one session-boundary for two sessions",
  );
  const boundaryIdx = blocks.indexOf(boundaryBlocks[0]);

  // Both system-prompt blocks must be present.
  const systemPromptBlocks = blocks.filter(
    (b) => b.kind === "single" && b.item?.acpSource === "session/new",
  );
  assert.equal(
    systemPromptBlocks.length,
    2,
    "both session/new markers must produce standalone single blocks",
  );

  const spAIdx = blocks.findIndex(
    (b) => b.kind === "single" && b.item?.id === "sp-a",
  );
  const spBIdx = blocks.findIndex(
    (b) => b.kind === "single" && b.item?.id === "sp-b",
  );
  const toolBIdx = blocks.findIndex((b) =>
    flattenDisplayBlocks([b]).some((i) => i.id === "toolB"),
  );

  // Both system-prompt blocks appear AFTER the boundary.
  assert.ok(
    boundaryIdx < spAIdx,
    `boundary (${boundaryIdx}) must precede sp-a (${spAIdx})`,
  );
  assert.ok(
    boundaryIdx < spBIdx,
    `boundary (${boundaryIdx}) must precede sp-b (${spBIdx})`,
  );

  // Both system-prompt blocks appear BEFORE toolB's turn block.
  assert.ok(
    spAIdx < toolBIdx,
    `sp-a (${spAIdx}) must precede toolB (${toolBIdx})`,
  );
  assert.ok(
    spBIdx < toolBIdx,
    `sp-b (${spBIdx}) must precede toolB (${toolBIdx})`,
  );

  // sp-a must appear before sp-b (wire order preserved).
  assert.ok(
    spAIdx < spBIdx,
    `sp-a (${spAIdx}) must precede sp-b (${spBIdx}) — wire order must be preserved`,
  );

  // The interleaved null-session frame must survive and appear in the sess-2 run
  // (after the boundary). This proves the "marker plus trailing null-session items"
  // contract — the null frame lands in the pending buffer between sp-a and sp-b
  // and must not be lost when the second marker arrives.
  const flat = flattenDisplayBlocks(blocks);
  const nullFrameInFlat = flat.some((i) => i.id === "null-frame");
  assert.ok(
    nullFrameInFlat,
    "interleaved null-session frame must survive in the flattened output",
  );
  const nullFrameBlockIdx = blocks.findIndex((b) =>
    flattenDisplayBlocks([b]).some((i) => i.id === "null-frame"),
  );
  assert.ok(
    boundaryIdx < nullFrameBlockIdx,
    `null-session frame (${nullFrameBlockIdx}) must appear after boundary (${boundaryIdx}) — it belongs to the new session run`,
  );
  // Wire order: sp-a → null-frame → sp-b (all flush in insertion order).
  const flatSpAIdx = flat.findIndex((i) => i.id === "sp-a");
  const flatNullIdx = flat.findIndex((i) => i.id === "null-frame");
  const flatSpBIdx = flat.findIndex((i) => i.id === "sp-b");
  assert.ok(
    flatSpAIdx < flatNullIdx,
    `sp-a (${flatSpAIdx}) must precede null-frame (${flatNullIdx}) in flat output`,
  );
  assert.ok(
    flatNullIdx < flatSpBIdx,
    `null-frame (${flatNullIdx}) must precede sp-b (${flatSpBIdx}) in flat output`,
  );

  // toolA (sess-1) must appear before the boundary.
  const toolAIdx = blocks.findIndex((b) =>
    flattenDisplayBlocks([b]).some((i) => i.id === "toolA"),
  );
  assert.ok(
    toolAIdx < boundaryIdx,
    `toolA (${toolAIdx}) must be before boundary (${boundaryIdx})`,
  );
});

// ── Regression suite for pendingForTurn / openBatch sealing ─────────────────

// Production wire order: turn_started(t1) → session/new → session_resolved(t1).
// The session/new arrives AFTER the bucket for turn-1 already exists.
// The system-prompt must still be hoisted before turn-1.
test("buildTranscriptDisplayBlocks_productionWireOrder_systemPromptBeforeTurn1", () => {
  const ts = "2026-07-08T12:00:00.000Z";
  const items = [
    // turn_started creates the bucket BEFORE session/new fires.
    {
      id: "turn-started-t1",
      type: "lifecycle",
      renderClass: "lifecycle",
      title: "Turn started",
      text: "",
      timestamp: ts,
      acpSource: "turn_started",
      turnId: "turn-1",
      sessionId: "sess-1",
      channelId: "chan-1",
    },
    // session/new fires AFTER the bucket exists (stale-stamped session).
    systemPromptItem("sp", "sess-0", ts),
    // session_resolved reuses the existing turn-1 bucket.
    {
      id: "session-resolved-t1",
      type: "lifecycle",
      renderClass: "lifecycle",
      title: "Session resolved",
      text: "",
      timestamp: ts,
      acpSource: "session_resolved",
      turnId: "turn-1",
      sessionId: "sess-1",
      channelId: "chan-1",
    },
    // A real activity item so the turn block has segments and appears in output.
    {
      id: "assistant-t1",
      type: "message",
      role: "assistant",
      title: "Assistant",
      text: "Hello",
      timestamp: ts,
      acpSource: "agent_message_chunk",
      turnId: "turn-1",
      sessionId: "sess-1",
      channelId: "chan-1",
    },
  ];

  const blocks = buildTranscriptDisplayBlocks(items);

  // sp must appear as a standalone single block.
  const spBlockIdx = blocks.findIndex(
    (b) => b.kind === "single" && b.item?.id === "sp",
  );
  // turn-1 must appear as a turn block.
  const turnBlockIdx = blocks.findIndex(
    (b) => b.kind === "turn" && b.turnId === "turn-1",
  );

  assert.ok(spBlockIdx !== -1, "sp block must be present");
  assert.ok(turnBlockIdx !== -1, "turn-1 block must be present");

  // System-prompt block must appear before the turn-1 block.
  assert.ok(
    spBlockIdx < turnBlockIdx,
    `sp block (${spBlockIdx}) must precede turn-1 block (${turnBlockIdx}) — system-prompt must flush before its anchor turn even when bucket pre-existed`,
  );
});

// Later null-session items (after turn-1) must NOT be hoisted before turn-1.
// Sequence: sp(null) → turn-1 item → later-null-frame(null) → turn-2 item
test("buildTranscriptDisplayBlocks_laterNullFrameAfterTurn1_notHoistedBeforeTurn1", () => {
  const ts = "2026-07-08T12:00:00.000Z";
  const items = [
    systemPromptItem("sp", "sess-1", ts),
    { ...sessionItem("toolA", "sess-1", ts), turnId: "turn-1" },
    {
      id: "later-null-frame",
      type: "lifecycle",
      renderClass: "lifecycle",
      title: "Session resolved",
      text: "",
      timestamp: ts,
      acpSource: "session_resolved",
      turnId: null,
      sessionId: "sess-1",
      channelId: "chan-1",
    },
    { ...sessionItem("toolB", "sess-1", ts), turnId: "turn-2" },
  ];

  const blocks = buildTranscriptDisplayBlocks(items);
  const flat = flattenDisplayBlocks(blocks);

  const spIdx = flat.findIndex((i) => i.id === "sp");
  const toolAIdx = flat.findIndex((i) => i.id === "toolA");
  const laterNullIdx = flat.findIndex((i) => i.id === "later-null-frame");
  const toolBIdx = flat.findIndex((i) => i.id === "toolB");

  assert.ok(spIdx !== -1, "sp must be present");
  assert.ok(toolAIdx !== -1, "toolA must be present");
  assert.ok(laterNullIdx !== -1, "later-null-frame must be present");
  assert.ok(toolBIdx !== -1, "toolB must be present");

  assert.ok(spIdx < toolAIdx, `sp (${spIdx}) must precede toolA (${toolAIdx})`);
  assert.ok(
    toolAIdx < laterNullIdx,
    `toolA (${toolAIdx}) must precede later-null-frame (${laterNullIdx}) — post-turn null items must NOT be hoisted`,
  );
  assert.ok(
    laterNullIdx < toolBIdx,
    `later-null-frame (${laterNullIdx}) must precede toolB (${toolBIdx})`,
  );
});

// A leading non-system-prompt null-session item before any session/new must be
// emitted inline and must not prevent subsequent system-prompt hoisting. The
// buffer-open decision must not depend on displayOrder.length === 0.
test("buildTranscriptDisplayBlocks_leadingSingleBeforeSessionNew_emittedInline", () => {
  const ts = "2026-07-08T12:00:00.000Z";
  const items = [
    {
      id: "early-lifecycle",
      type: "lifecycle",
      renderClass: "lifecycle",
      title: "Agent connected",
      text: "",
      timestamp: ts,
      acpSource: "agent_connected",
      turnId: null,
      sessionId: "sess-1",
      channelId: "chan-1",
    },
    systemPromptItem("sp", "sess-1", ts),
    { ...sessionItem("toolA", "sess-1", ts), turnId: "turn-1" },
  ];

  const blocks = buildTranscriptDisplayBlocks(items);
  const flat = flattenDisplayBlocks(blocks);

  const earlyIdx = flat.findIndex((i) => i.id === "early-lifecycle");
  const spIdx = flat.findIndex((i) => i.id === "sp");
  const toolAIdx = flat.findIndex((i) => i.id === "toolA");

  assert.ok(earlyIdx !== -1, "early-lifecycle must be present");
  assert.ok(spIdx !== -1, "sp must be present");
  assert.ok(toolAIdx !== -1, "toolA must be present");

  assert.ok(
    earlyIdx < toolAIdx,
    `early-lifecycle (${earlyIdx}) must precede toolA (${toolAIdx})`,
  );
  assert.ok(
    spIdx < toolAIdx,
    `sp (${spIdx}) must precede toolA (${toolAIdx}) — system-prompt must be hoisted even after a leading item`,
  );
});
