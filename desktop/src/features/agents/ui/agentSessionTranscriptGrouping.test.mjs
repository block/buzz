import assert from "node:assert/strict";
import test from "node:test";

import {
  buildTranscriptDisplayBlocks,
  flattenDisplayBlocks,
  formatTurnSetupLabel,
} from "./agentSessionTranscriptGrouping.ts";

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

test("buildTranscriptDisplayBlocks keeps lifecycle visible when prompt is missing", () => {
  const rawItems = [
    lifecycle("turn", "Turn started", "turn_started", "turn-1"),
    lifecycle("session", "Session ready", "session_resolved", "turn-1"),
  ];

  const blocks = buildTranscriptDisplayBlocks(rawItems);
  const displayOrder = flattenDisplayBlocks(blocks).map((item) => item.id);

  assert.deepEqual(displayOrder, ["turn", "session"]);
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
