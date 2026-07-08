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
  assert.equal(steerSegment.systemPrompt, null);
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
